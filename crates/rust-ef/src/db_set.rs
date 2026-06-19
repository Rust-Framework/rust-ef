//! DbSet<T> — entry point for querying and manipulating entity collections.
//!
//! `DbSet<T>` represents a typed collection of entities that can be queried
//! and mutated. It implements two interfaces following ISP:
//!   - `IQueryable<T>` — query capabilities
//!   - `IDbSet<T>`     — collection mutation capabilities

use crate::entity::{EntityState, IEntityType};
use crate::error::LrefResult;
use crate::provider::IDatabaseProvider;
use crate::query::{IQueryable, QueryBuilder};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// IDbSet<T> — interface for entity collection manipulation
// ---------------------------------------------------------------------------

/// Interface for manipulating a typed entity collection.
///
/// Separates mutation concerns from query concerns (see `IQueryable<T>`).
/// Implemented by `DbSet<T>`.
pub trait IDbSet<T: IEntityType>: IQueryable<T> + Send + Sync {
    /// Adds a new entity to the set in Added state.
    fn add(&mut self, entity: T);

    /// Marks all entities in the set as Deleted.
    fn remove_all(&mut self);

    /// Marks the entity at the given index as Deleted.
    fn remove_at(&mut self, index: usize) -> LrefResult<()>;

    /// Attaches an existing entity in Unchanged state.
    fn attach(&mut self, entity: T);

    /// Returns references to entities in Added state.
    fn added_entities(&self) -> Vec<&T>;

    /// Returns references to entities in Modified state.
    fn modified_entities(&self) -> Vec<&T>;

    /// Returns references to entities in Deleted state.
    fn deleted_entities(&self) -> Vec<&T>;

    /// Returns an iterator over entity references and their states.
    fn entries_with_state(&self) -> Vec<(&T, EntityState)>;

    /// Clears all tracked entries from the set.
    fn clear_entries(&mut self);

    /// Returns the number of tracked entries.
    fn len(&self) -> usize;

    /// Returns whether the set is empty.
    fn is_empty(&self) -> bool;
}

// ---------------------------------------------------------------------------
// DbSet<T> — concrete implementation
// ---------------------------------------------------------------------------

pub struct DbSet<T: IEntityType> {
    pub(crate) entries: Vec<TrackedEntry<T>>,
    table_name: String,
    provider: Option<Arc<dyn IDatabaseProvider>>,
}

pub struct TrackedEntry<T: IEntityType> {
    pub entity: T,
    pub state: EntityState,
}

impl<T: IEntityType> DbSet<T> {
    pub fn new(table_name: impl Into<String>) -> Self {
        Self {
            entries: Vec::new(),
            table_name: table_name.into(),
            provider: None,
        }
    }

    pub fn with_provider(
        table_name: impl Into<String>,
        provider: Arc<dyn IDatabaseProvider>,
    ) -> Self {
        Self {
            entries: Vec::new(),
            table_name: table_name.into(),
            provider: Some(provider),
        }
    }

    pub fn set_provider(&mut self, provider: Arc<dyn IDatabaseProvider>) {
        self.provider = Some(provider);
    }

    // ── Convenience inherent methods — delegate to trait implementations ──

    /// Convenience inherent method — delegates to `IDbSet::add`.
    pub fn add(&mut self, entity: T) {
        IDbSet::add(self, entity);
    }

    /// Convenience inherent method — delegates to `IDbSet::remove_all`.
    pub fn remove_all(&mut self) {
        IDbSet::remove_all(self);
    }

    /// Convenience inherent method — delegates to `IDbSet::remove_at`.
    pub fn remove_at(&mut self, index: usize) -> LrefResult<()> {
        IDbSet::remove_at(self, index)
    }

    /// Convenience inherent method — delegates to `IDbSet::clear_entries`.
    pub fn clear_entries(&mut self) {
        IDbSet::clear_entries(self);
    }

    /// Convenience inherent method — delegates to `IDbSet::len`.
    pub fn len(&self) -> usize {
        IDbSet::len(self)
    }

    /// Convenience inherent method — delegates to `IDbSet::is_empty`.
    pub fn is_empty(&self) -> bool {
        IDbSet::is_empty(self)
    }

    /// Convenience inherent method — delegates to `IQueryable::query`.
    pub fn query(&self) -> QueryBuilder<T> {
        IQueryable::query(self)
    }

    pub fn attach(&mut self, entity: T) {
        IDbSet::attach(self, entity);
    }

    pub fn tracked_entries(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|e| &e.entity)
    }

    pub fn tracked_entries_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|e| &mut e.entity)
    }

    pub fn retain(&mut self, f: impl FnMut(&TrackedEntry<T>) -> bool) {
        self.entries.retain(f);
    }
}

// ---------------------------------------------------------------------------
// IQueryable<T> implementation
// ---------------------------------------------------------------------------

impl<T: IEntityType> IQueryable<T> for DbSet<T> {
    fn query(&self) -> QueryBuilder<T> {
        match &self.provider {
            Some(p) => QueryBuilder::with_provider(&self.table_name, p.clone()),
            None => QueryBuilder::new(&self.table_name),
        }
    }
}

// ---------------------------------------------------------------------------
// IDbSet<T> implementation
// ---------------------------------------------------------------------------

impl<T: IEntityType> IDbSet<T> for DbSet<T> {
    fn add(&mut self, entity: T) {
        self.entries.push(TrackedEntry {
            entity,
            state: EntityState::Added,
        });
    }

    fn remove_all(&mut self) {
        for entry in &mut self.entries {
            entry.state = EntityState::Deleted;
        }
    }

    fn remove_at(&mut self, index: usize) -> LrefResult<()> {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.state = EntityState::Deleted;
            Ok(())
        } else {
            Err(crate::error::LrefError::NotFound(
                "Entity not found at the given index".to_string(),
            ))
        }
    }

    fn attach(&mut self, entity: T) {
        self.entries.push(TrackedEntry {
            entity,
            state: EntityState::Unchanged,
        });
    }

    fn added_entities(&self) -> Vec<&T> {
        self.entries
            .iter()
            .filter(|e| e.state == EntityState::Added)
            .map(|e| &e.entity)
            .collect()
    }

    fn modified_entities(&self) -> Vec<&T> {
        self.entries
            .iter()
            .filter(|e| e.state == EntityState::Modified)
            .map(|e| &e.entity)
            .collect()
    }

    fn deleted_entities(&self) -> Vec<&T> {
        self.entries
            .iter()
            .filter(|e| e.state == EntityState::Deleted)
            .map(|e| &e.entity)
            .collect()
    }

    fn entries_with_state(&self) -> Vec<(&T, EntityState)> {
        self.entries.iter().map(|e| (&e.entity, e.state)).collect()
    }

    fn clear_entries(&mut self) {
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
