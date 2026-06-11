//! DbSet<T> — entry point for querying and manipulating entity collections.

use crate::entity::{EntityState, EntityType};
use crate::error::LrefResult;
use crate::provider::DatabaseProvider;
use crate::query::QueryBuilder;
use std::sync::Arc;

pub struct DbSet<T: EntityType> {
    pub(crate) entries: Vec<TrackedEntry<T>>,
    table_name: String,
    provider: Option<Arc<dyn DatabaseProvider>>,
}

pub struct TrackedEntry<T: EntityType> {
    pub entity: T,
    pub state: EntityState,
}

impl<T: EntityType> DbSet<T> {
    pub fn new(table_name: impl Into<String>) -> Self {
        Self {
            entries: Vec::new(),
            table_name: table_name.into(),
            provider: None,
        }
    }

    pub fn with_provider(table_name: impl Into<String>, provider: Arc<dyn DatabaseProvider>) -> Self {
        Self {
            entries: Vec::new(),
            table_name: table_name.into(),
            provider: Some(provider),
        }
    }

    pub fn set_provider(&mut self, provider: Arc<dyn DatabaseProvider>) {
        self.provider = Some(provider);
    }

    pub fn add(&mut self, entity: T) {
        self.entries.push(TrackedEntry { entity, state: EntityState::Added });
    }

    pub fn remove_all(&mut self) {
        for entry in &mut self.entries {
            entry.state = EntityState::Deleted;
        }
    }

    pub fn remove_at(&mut self, index: usize) -> LrefResult<()> {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.state = EntityState::Deleted;
            Ok(())
        } else {
            Err(crate::error::LrefError::NotFound(
                "Entity not found at the given index".to_string(),
            ))
        }
    }

    pub fn attach(&mut self, entity: T) {
        self.entries.push(TrackedEntry { entity, state: EntityState::Unchanged });
    }

    pub fn query(&self) -> QueryBuilder<T> {
        match &self.provider {
            Some(p) => QueryBuilder::with_provider(&self.table_name, p.clone()),
            None => QueryBuilder::new(&self.table_name),
        }
    }

    pub fn tracked_entries(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|e| &e.entity)
    }

    pub fn tracked_entries_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|e| &mut e.entity)
    }

    pub fn entries_with_state(&self) -> impl Iterator<Item = (&T, EntityState)> {
        self.entries.iter().map(|e| (&e.entity, e.state))
    }

    /// Returns references to entities in Added state.
    pub fn added_entities(&self) -> Vec<&T> {
        self.entries
            .iter()
            .filter(|e| e.state == EntityState::Added)
            .map(|e| &e.entity)
            .collect()
    }

    /// Returns references to entities in Modified state.
    pub fn modified_entities(&self) -> Vec<&T> {
        self.entries
            .iter()
            .filter(|e| e.state == EntityState::Modified)
            .map(|e| &e.entity)
            .collect()
    }

    /// Returns references to entities in Deleted state.
    pub fn deleted_entities(&self) -> Vec<&T> {
        self.entries
            .iter()
            .filter(|e| e.state == EntityState::Deleted)
            .map(|e| &e.entity)
            .collect()
    }

    pub fn clear_entries(&mut self) {
        self.entries.clear();
    }

    pub fn retain(&mut self, f: impl FnMut(&TrackedEntry<T>) -> bool) {
        self.entries.retain(f);
    }

    /// Returns the number of tracked entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
