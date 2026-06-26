//! SaveChanges interceptor �?hooks into the DbContext lifecycle.
//!
//! Provides an interception pipeline analogous to EFCore's
//! `ISaveChangesInterceptor`: before/after save, and on failure.
//! Interceptors are registered via `DbContextOptionsBuilder::add_interceptor()`.
//!
//! # Example (user code)
//!
//! ```rust,ignore
//! use rust_ef::interceptor::{ISaveChangesInterceptor, SaveChangesContext};
//!
//! struct AuditInterceptor;
//!
//! #[async_trait::async_trait]
//! impl ISaveChangesInterceptor for AuditInterceptor {
//!     async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
//!         println!("Saving {} entries", ctx.entries().len());
//!         Ok(())
//!     }
//! }
//! ```

use crate::error::{EFError, EFResult};
use crate::tracking::{ChangeTracker, EntityEntry};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// SaveChangesContext �?context passed to interceptor hooks
// ---------------------------------------------------------------------------

/// Read-only snapshot of the save operation, passed to interceptors.
///
/// Provides access to the change tracker state at the moment of
/// interception. This is an owned snapshot so interceptors cannot
/// mutate the actual pending changes, and the borrow does not block
/// subsequent mutating operations on `ChangeTracker`.
#[derive(Debug, Clone)]
pub struct SaveChangesContext {
    /// All tracked entries before the save.
    entries: Vec<EntityEntry>,
    /// Number of entries that will be added.
    added_count: usize,
    /// Number of entries that will be modified.
    modified_count: usize,
    /// Number of entries that will be deleted.
    deleted_count: usize,
}

impl SaveChangesContext {
    /// Creates a context from the current change tracker state.
    ///
    /// This takes an owned snapshot �?no borrow is retained.
    pub fn from_tracker(tracker: &ChangeTracker) -> Self {
        let entries = tracker.entries();
        let added_count = tracker.count_by_state(crate::entity::EntityState::Added);
        let modified_count = tracker.count_by_state(crate::entity::EntityState::Modified);
        let deleted_count = tracker.count_by_state(crate::entity::EntityState::Deleted);

        Self {
            entries,
            added_count,
            modified_count,
            deleted_count,
        }
    }

    /// Returns all tracked entity entries at the time of interception.
    pub fn entries(&self) -> &[EntityEntry] {
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
// SaveChangesResultContext �?post-save context
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
// ISaveChangesInterceptor �?the interceptor trait
// ---------------------------------------------------------------------------

/// Interface for intercepting `save_changes()` operations.
///
/// All methods have default no-op implementations so users only
/// override what they need.
///
/// # Lifecycle
///
/// ```text
/// on_saving(ctx) �?[execute SQL] �?on_saved(ctx, result)
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
// InterceptorPipeline �?ordered chain of interceptors
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

    /// Runs all `on_saving` hooks in registration order.
    /// Aborts early if any interceptor returns an error.
    pub async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        for interceptor in &self.interceptors {
            interceptor.on_saving(ctx).await?;
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
    /// intentionally swallowed �?they must not mask the original error.
    pub async fn on_save_failed(&self, ctx: &SaveChangesContext, error: &EFError) {
        for interceptor in &self.interceptors {
            interceptor.on_save_failed(ctx, error).await;
        }
    }
}
