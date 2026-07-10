//! `DbSet<T>` — typed entity collection for querying and storage.
//!
//! `DbSet<T>` holds typed entities + `entry_id` links to `ChangeTracker`.
//! Tracking state (EntityState, original snapshot, modified properties,
//! is_upsert) lives in `ChangeTracker`, joined by `entry_id` during save.
//! Mutations (`add`, `attach`, `update`, `upsert`, `remove`) go through
//! `DbContext` methods which coordinate DbSet + ChangeTracker.

use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use crate::error::EFResult;
use crate::provider::{DbValue, IDatabaseProvider};
use crate::query::{BoolExpr, IQueryable, QueryBuilder};
use std::collections::HashMap;
use std::sync::Arc;

/// A single entry in a `DbSet` — the typed entity + its `ChangeTracker` id.
///
/// Tracking state is NOT stored here. Use `ChangeTracker::entry_state(id)` /
/// `entry_original(id)` / `entry_modified(id)` / `entry_is_upsert(id)` to
/// query tracking state, joined by `entry_id`.
pub struct DbSetEntry<T: IEntityType> {
    pub entity: T,
    pub entry_id: u64,
}

/// Collection-level operations on a typed entity collection.
///
/// Mutation methods (`add`, `attach`, `update`, `upsert`, `remove`) are on
/// `DbContext`, not here — they need to coordinate with `ChangeTracker`.
pub trait IDbSet<T: IEntityType>: IQueryable<T> + Send + Sync {
    /// Clears all entries from the set (does not affect ChangeTracker).
    fn clear_entries(&mut self);

    /// Returns the number of entries.
    fn len(&self) -> usize;

    /// Returns whether the set is empty.
    fn is_empty(&self) -> bool;
}

pub struct DbSet<T: IEntityType> {
    pub(crate) entries: Vec<DbSetEntry<T>>,
    table_name: String,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    query_filter: Option<BoolExpr>,
    filter_map: Option<Arc<HashMap<String, crate::query::CompiledFilter>>>,
    lazy_loading_enabled: bool,
}

impl<T: IEntityType + IEntitySnapshot> DbSet<T> {
    pub fn new(table_name: impl Into<String>) -> Self {
        Self {
            entries: Vec::new(),
            table_name: table_name.into(),
            provider: None,
            query_filter: None,
            filter_map: None,
            lazy_loading_enabled: false,
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
            query_filter: None,
            filter_map: None,
            lazy_loading_enabled: false,
        }
    }

    pub fn set_query_filter(&mut self, filter: BoolExpr) {
        self.query_filter = Some(filter);
    }

    /// Sets the global filter map (table_name → BoolExpr) used by
    /// NavigationLoader to scope secondary queries.
    pub fn set_filter_map(&mut self, map: Arc<HashMap<String, crate::query::CompiledFilter>>) {
        self.filter_map = Some(map);
    }

    /// Propagates the lazy-loading flag from `DbContextOptions` to this set.
    pub(crate) fn set_lazy_loading_enabled(&mut self, enabled: bool) {
        self.lazy_loading_enabled = enabled;
    }

    /// Returns the configured query filter, if any. Used by save phases
    /// to apply tenant isolation to UPDATE/DELETE WHERE clauses.
    pub(crate) fn query_filter(&self) -> Option<&BoolExpr> {
        self.query_filter.as_ref()
    }

    pub fn set_provider(&mut self, provider: Arc<dyn IDatabaseProvider>) {
        self.provider = Some(provider);
    }

    // ── Internal: entry management (used by DbContext mutations) ──

    /// Pushes a new entry with a pre-assigned `entry_id` from ChangeTracker.
    pub(crate) fn push_entry(&mut self, entity: T, entry_id: u64) {
        self.entries.push(DbSetEntry { entity, entry_id });
    }

    /// Returns the `entry_id` at the given index, or `None` if out of bounds.
    pub(crate) fn entry_id_at(&self, idx: usize) -> Option<u64> {
        self.entries.get(idx).map(|e| e.entry_id)
    }

    // ── Collection access ──

    /// Returns an iterator over entity references.
    pub fn tracked_entries(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|e| &e.entity)
    }

    /// Returns a mutable iterator over entity references.
    pub fn tracked_entries_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|e| &mut e.entity)
    }

    /// Retains entries matching the predicate (drops others).
    pub fn retain(&mut self, f: impl FnMut(&DbSetEntry<T>) -> bool) {
        self.entries.retain(f);
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

    // ── Query methods ──

    /// Convenience inherent method — delegates to `IQueryable::query`.
    pub fn query(&self) -> QueryBuilder<T> {
        IQueryable::query(self)
    }

    /// Returns a query builder that bypasses the configured query filter.
    /// Use for administrative / cross-tenant queries.
    pub fn query_ignore_filters(&self) -> QueryBuilder<T> {
        let qb = match &self.provider {
            Some(p) => QueryBuilder::with_provider(&self.table_name, p.clone()),
            None => QueryBuilder::new(&self.table_name),
        };
        qb.with_filter_map(self.filter_map.clone())
            .with_lazy_loading(self.lazy_loading_enabled)
    }

    /// Starts a query filtered by a compile-time LINQ expression tree (`linq!()`)
    pub fn filter<F>(&self, apply: F) -> QueryBuilder<T>
    where
        F: FnOnce(QueryBuilder<T>) -> QueryBuilder<T>,
    {
        apply(self.query())
    }

    /// Checks whether an entity with the given key values exists in the database.
    pub async fn exists_by_id(&self, key_values: HashMap<String, DbValue>) -> EFResult<bool>
    where
        T: IFromRow
            + INavigationSetter
            + IGetKeyValues
            + IEntitySnapshot
            + crate::entity::ILazyInit,
    {
        let pairs: Vec<(&str, DbValue)> = key_values
            .iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect();
        Ok(self.query().find_by_key(&pairs).await?.is_some())
    }
}

// ---------------------------------------------------------------------------
// IQueryable<T> implementation
// ---------------------------------------------------------------------------

impl<T: IEntityType> IQueryable<T> for DbSet<T> {
    fn query(&self) -> QueryBuilder<T> {
        let mut qb = match &self.provider {
            Some(p) => QueryBuilder::with_provider(&self.table_name, p.clone()),
            None => QueryBuilder::new(&self.table_name),
        };
        if let Some(ref filter) = self.query_filter {
            qb = qb.apply_query_filter(filter.clone());
        }
        qb.with_filter_map(self.filter_map.clone())
            .with_lazy_loading(self.lazy_loading_enabled)
    }
}

// ---------------------------------------------------------------------------
// IDbSet<T> implementation
// ---------------------------------------------------------------------------

impl<T: IEntityType + IEntitySnapshot> IDbSet<T> for DbSet<T> {
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
