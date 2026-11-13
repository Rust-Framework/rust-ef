//! SaveChanges interceptor — hooks into the DbContext lifecycle.
//!
//! Provides an interception pipeline analogous to EFCore's
//! `ISaveChangesInterceptor`: before/after save, and on failure.
//! Interceptors are registered via `DbContextOptionsBuilder::add_interceptor()`.
//!
//! Two pre-save hooks exist:
//! - [`ISaveChangesInterceptor::on_saving`] gets a read-only
//!   [`SaveChangesContext`] snapshot.
//! - [`ISaveChangesInterceptor::on_saving_mut`] gets a mutable
//!   [`MutableSaveChangesContext`], so it can write values that must be
//!   committed together with the save (audit timestamps/actors, normalized
//!   fields, derived columns). The mutated entities are then re-detected and
//!   written by the save's own transaction.
//!
//! # Example (user code)
//!
//! ```rust,ignore
//! use rust_ef::interceptor::{
//!     ISaveChangesInterceptor, MutableSaveChangesContext, SaveChangesContext,
//! };
//!
//! struct AuditInterceptor;
//!
//! #[async_trait::async_trait]
//! impl ISaveChangesInterceptor for AuditInterceptor {
//!     async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
//!         println!("Saving {} entries", ctx.entries().len());
//!         Ok(())
//!     }
//!
//!     async fn on_saving_mut(&self, ctx: &mut MutableSaveChangesContext<'_>) -> EFResult<()> {
//!         let now = "2026-11-13T00:00:00Z";
//!         ctx.for_each_entity_mut::<Order>(|state, order| {
//!             if state == EntityState::Added {
//!                 order.created_at = now.into();
//!             }
//!             order.updated_at = now.into();
//!         });
//!         Ok(())
//!     }
//! }
//! ```

use crate::entity::EntityState;
use crate::error::{EFError, EFResult};
use crate::tracking::EntityEntryView;
use std::any::{Any, TypeId};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// SaveChangesContext — context passed to interceptor hooks
// ---------------------------------------------------------------------------

/// Read-only snapshot of the save operation, passed to interceptors.
///
/// Built from the actual pending entries across all `DbSet`s (the real save
/// data source), so interceptors see a snapshot consistent with what
/// `save_changes()` will commit. This is an owned snapshot so the read-only
/// hook cannot mutate the pending changes; interceptors that must adjust
/// entities before the commit use
/// [`ISaveChangesInterceptor::on_saving_mut`] with a
/// [`MutableSaveChangesContext`] instead.
#[derive(Debug, Clone)]
pub struct SaveChangesContext {
    /// All pending entries before the save.
    entries: Vec<EntityEntryView>,
    /// Number of entries that will be added.
    added_count: usize,
    /// Number of entries that will be modified.
    modified_count: usize,
    /// Number of entries that will be deleted.
    deleted_count: usize,
}

impl SaveChangesContext {
    /// Creates a context from the pending entries aggregated across all
    /// `DbSet`s. Counts are derived from each entry's `state`.
    pub fn from_views(entries: Vec<EntityEntryView>) -> Self {
        let added_count = entries
            .iter()
            .filter(|e| e.state == crate::entity::EntityState::Added)
            .count();
        let modified_count = entries
            .iter()
            .filter(|e| e.state == crate::entity::EntityState::Modified)
            .count();
        let deleted_count = entries
            .iter()
            .filter(|e| e.state == crate::entity::EntityState::Deleted)
            .count();

        Self {
            entries,
            added_count,
            modified_count,
            deleted_count,
        }
    }

    /// Returns all pending entity entries at the time of interception.
    pub fn entries(&self) -> &[EntityEntryView] {
        &self.entries
    }

    /// Number of entities that will be / were added.
    pub fn added_count(&self) -> usize {
        self.added_count
    }

    /// Number of entities that will be / were modified.
    pub fn modified_count(&self) -> usize {
        self.modified_count
    }

    /// Number of entities that will be / were deleted.
    pub fn deleted_count(&self) -> usize {
        self.deleted_count
    }

    /// Total number of tracked entries.
    pub fn total_count(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// SaveChangesResultContext — post-save context
// ---------------------------------------------------------------------------

/// Context passed to interceptors after a successful save.
#[derive(Debug, Clone)]
pub struct SaveChangesResultContext {
    /// Summary of the save operation.
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
}

impl SaveChangesResultContext {
    pub fn total(&self) -> usize {
        self.added + self.updated + self.deleted
    }
}

// ---------------------------------------------------------------------------
// Mutable pre-save context — interceptors that adjust entities
// ---------------------------------------------------------------------------

/// Mutable, type-erased view of one pending entity entry.
///
/// Passed to [`ISaveChangesInterceptor::on_saving_mut`]. The entity is the
/// actual tracked instance, so writes made through [`entity_mut`](Self::entity_mut)
/// are part of the save that is about to run.
pub struct MutableEntityEntry<'a> {
    state: EntityState,
    type_id: TypeId,
    type_name: String,
    entity: &'a mut (dyn Any + Send + Sync),
}

impl<'a> MutableEntityEntry<'a> {
    pub(crate) fn new(
        state: EntityState,
        type_id: TypeId,
        type_name: String,
        entity: &'a mut (dyn Any + Send + Sync),
    ) -> Self {
        Self {
            state,
            type_id,
            type_name,
            entity,
        }
    }

    /// Tracking state at the time the hook runs (`Added`, `Unchanged`,
    /// `Modified`, `Deleted`; change detection has not run yet, so a tracked
    /// existing entity is `Unchanged` until the save detects its edits).
    pub fn state(&self) -> EntityState {
        self.state
    }

    /// Concrete entity type id (`TypeId::of::<T>()`).
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Concrete entity type name, for diagnostics.
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Type-erased shared access to the entity.
    pub fn entity(&self) -> &(dyn Any + Send + Sync) {
        self.entity
    }

    /// Downcasts to the concrete entity type, or `None` if this entry tracks a
    /// different type.
    pub fn entity_ref<T: Any>(&self) -> Option<&T> {
        self.entity.downcast_ref::<T>()
    }

    /// Downcasts to the concrete entity type **mutably**, or `None` if this
    /// entry tracks a different type.
    ///
    /// Writes made here are committed together with the save.
    pub fn entity_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.entity.downcast_mut::<T>()
    }
}

/// Mutable view of the pending changes, passed to
/// [`ISaveChangesInterceptor::on_saving_mut`].
///
/// The hook runs before the save's transaction and before change detection:
/// mutations made here are what the save reads, detects and writes, so they are
/// committed atomically with the save (never in a separate transaction).
pub struct MutableSaveChangesContext<'a> {
    entries: Vec<MutableEntityEntry<'a>>,
}

impl<'a> MutableSaveChangesContext<'a> {
    pub(crate) fn new(entries: Vec<MutableEntityEntry<'a>>) -> Self {
        Self { entries }
    }

    /// All pending entries.
    pub fn entries(&self) -> &[MutableEntityEntry<'a>] {
        &self.entries
    }

    /// All pending entries, mutable.
    pub fn entries_mut(&mut self) -> &mut [MutableEntityEntry<'a>] {
        &mut self.entries
    }

    /// Total number of pending entries.
    pub fn total_count(&self) -> usize {
        self.entries.len()
    }

    /// Applies `f` to every pending entity of type `T`, passing its tracking
    /// state. Returns the number of entities visited.
    ///
    /// This is the ergonomic form for cross-cutting entity mutation, e.g.
    /// audit timestamps:
    ///
    /// ```rust,ignore
    /// ctx.for_each_entity_mut::<Order>(|state, order| {
    ///     if state == EntityState::Added {
    ///         order.created_at = now.clone();
    ///     }
    ///     order.updated_at = now.clone();
    /// });
    /// ```
    pub fn for_each_entity_mut<T: Any>(&mut self, mut f: impl FnMut(EntityState, &mut T)) -> usize {
        let mut visited = 0;
        for entry in &mut self.entries {
            let state = entry.state;
            if let Some(entity) = entry.entity_mut::<T>() {
                f(state, entity);
                visited += 1;
            }
        }
        visited
    }
}

// ---------------------------------------------------------------------------
// ISaveChangesInterceptor — the interceptor trait
// ---------------------------------------------------------------------------

/// Interface for intercepting `save_changes()` operations.
///
/// All methods have default no-op implementations so users only
/// override what they need.
///
/// # Lifecycle
///
/// ```text
/// on_saving(ctx) — [execute SQL] — on_saved(ctx, result)
///                                    on_save_failed(ctx, error)  (if error)
/// ```
#[async_trait::async_trait]
pub trait ISaveChangesInterceptor: Send + Sync {
    /// Called **before** SQL commands are executed.
    ///
    /// Return `Err(...)` to abort the save. The transaction will be
    /// rolled back and `on_save_failed` will NOT be called (this is
    /// a pre-commit rejection, not a database failure).
    async fn on_saving(&self, _ctx: &SaveChangesContext) -> EFResult<()> {
        Ok(())
    }

    /// Called **before** SQL commands are executed, with mutable access to the
    /// pending entities.
    ///
    /// Mutations made here are what the save reads, detects and writes: they
    /// are committed inside the **same transaction** as the rest of the save.
    /// Use it for values the server must own (audit timestamps/actors,
    /// normalized fields, derived columns) — request/body values can be
    /// overwritten here before they reach the database.
    ///
    /// Return `Err(...)` to abort the save (same pre-commit rejection
    /// semantics as [`on_saving`](Self::on_saving)).
    async fn on_saving_mut(&self, _ctx: &mut MutableSaveChangesContext<'_>) -> EFResult<()> {
        Ok(())
    }

    /// Called **after** a successful save (transaction committed).
    async fn on_saved(
        &self,
        _ctx: &SaveChangesContext,
        _result: &SaveChangesResultContext,
    ) -> EFResult<()> {
        Ok(())
    }

    /// Called when the save **fails** with a database error.
    ///
    /// The transaction has already been rolled back at this point.
    /// The error is passed by reference for inspection; the interceptor
    /// cannot suppress it.
    async fn on_save_failed(&self, _ctx: &SaveChangesContext, _error: &EFError) {
        // default: no-op
    }
}

// ---------------------------------------------------------------------------
// InterceptorPipeline — ordered chain of interceptors
// ---------------------------------------------------------------------------

/// An ordered, immutable chain of save-changes interceptors.
///
/// Created by `DbContextOptionsBuilder` and invoked by `DbContext::save_changes()`.
pub(crate) struct InterceptorPipeline {
    interceptors: Vec<Arc<dyn ISaveChangesInterceptor>>,
}

impl InterceptorPipeline {
    pub fn new(interceptors: Vec<Arc<dyn ISaveChangesInterceptor>>) -> Self {
        Self { interceptors }
    }

    /// Returns `true` when at least one interceptor is registered. Callers use
    /// this to skip building the mutable pre-save context (which allocates one
    /// type-erased entry per tracked entity) on the common no-interceptor path.
    pub fn has_interceptors(&self) -> bool {
        !self.interceptors.is_empty()
    }

    /// Runs all `on_saving` hooks in registration order.
    /// Aborts early if any interceptor returns an error.
    pub async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        for interceptor in &self.interceptors {
            interceptor.on_saving(ctx).await?;
        }
        Ok(())
    }

    /// Runs all `on_saving_mut` hooks in registration order.
    /// Aborts early if any interceptor returns an error.
    pub async fn on_saving_mut(&self, ctx: &mut MutableSaveChangesContext<'_>) -> EFResult<()> {
        for interceptor in &self.interceptors {
            interceptor.on_saving_mut(ctx).await?;
        }
        Ok(())
    }

    /// Runs all `on_saved` hooks in registration order.
    pub async fn on_saved(
        &self,
        ctx: &SaveChangesContext,
        result: &SaveChangesResultContext,
    ) -> EFResult<()> {
        for interceptor in &self.interceptors {
            interceptor.on_saved(ctx, result).await?;
        }
        Ok(())
    }

    /// Runs all `on_save_failed` hooks. Errors from these hooks are
    /// intentionally swallowed — they must not mask the original error.
    pub async fn on_save_failed(&self, ctx: &SaveChangesContext, error: &EFError) {
        for interceptor in &self.interceptors {
            interceptor.on_save_failed(ctx, error).await;
        }
    }
}
