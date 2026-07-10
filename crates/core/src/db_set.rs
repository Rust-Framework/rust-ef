//! DbSet<T> �?entry point for querying and manipulating entity collections.
//!
//! `DbSet<T>` represents a typed collection of entities that can be queried
//! and mutated. It implements two interfaces following ISP:
//!   - `IQueryable<T>` �?query capabilities
//!   - `IDbSet<T>`     �?collection mutation capabilities

use crate::entity::{
    EntityState, IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter,
};
use crate::entity_snapshot::EntitySnapshot;
use crate::error::EFResult;
use crate::provider::{DbValue, IDatabaseProvider};
use crate::query::{BoolExpr, IQueryable, QueryBuilder};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// IDbSet<T> �?interface for entity collection manipulation
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
    fn remove_at(&mut self, index: usize) -> EFResult<()>;

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
// DbSet<T> �?concrete implementation
// ---------------------------------------------------------------------------

pub struct DbSet<T: IEntityType> {
    pub(crate) entries: Vec<TrackedEntry<T>>,
    table_name: String,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    query_filter: Option<BoolExpr>,
    filter_map: Option<Arc<HashMap<String, crate::query::CompiledFilter>>>,
    lazy_loading_enabled: bool,
}

pub struct TrackedEntry<T: IEntityType> {
    pub entity: T,
    pub state: EntityState,
    /// Snapshot taken when the entity was attached (for change detection).
    pub original: Option<EntitySnapshot>,
    /// Field names that differ from `original` (populated by `detect_changes`).
    /// Empty when `detect_changes` hasn't run or the entity was marked Modified
    /// directly via `update()`. Used by `execute_updates` to generate partial
    /// UPDATE statements (SET only dirty columns).
    pub modified_properties: Vec<String>,
    /// When `true`, the entry is in Added state but should be saved via
    /// `execute_upserts` (INSERT ... ON CONFLICT DO UPDATE) instead of a plain
    /// INSERT. Set by `upsert()`.
    pub is_upsert: bool,
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

    /// Returns the configured query filter, if any. Used by `save_one_set`
    /// to apply tenant isolation to UPDATE/DELETE WHERE clauses.
    pub(crate) fn query_filter(&self) -> Option<&BoolExpr> {
        self.query_filter.as_ref()
    }

    pub fn set_provider(&mut self, provider: Arc<dyn IDatabaseProvider>) {
        self.provider = Some(provider);
    }

    // ── Convenience inherent methods �?delegate to trait implementations ──

    /// Convenience inherent method �?delegates to `IDbSet::add`.
    pub fn add(&mut self, entity: T) {
        IDbSet::add(self, entity);
    }

    /// Convenience inherent method �?delegates to `IDbSet::remove_all`.
    pub fn remove_all(&mut self) {
        IDbSet::remove_all(self);
    }

    /// Convenience inherent method �?delegates to `IDbSet::remove_at`.
    pub fn remove_at(&mut self, index: usize) -> EFResult<()> {
        IDbSet::remove_at(self, index)
    }

    /// Convenience inherent method �?delegates to `IDbSet::clear_entries`.
    pub fn clear_entries(&mut self) {
        IDbSet::clear_entries(self);
    }

    /// Convenience inherent method �?delegates to `IDbSet::len`.
    pub fn len(&self) -> usize {
        IDbSet::len(self)
    }

    /// Convenience inherent method �?delegates to `IDbSet::is_empty`.
    pub fn is_empty(&self) -> bool {
        IDbSet::is_empty(self)
    }

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

    /// Marks multiple entities as deleted.
    pub fn remove_range(&mut self, entities: &[T])
    where
        T: PartialEq,
    {
        for entity in entities {
            if let Some(entry) = self.entries.iter_mut().find(|e| e.entity == *entity) {
                entry.state = EntityState::Deleted;
            }
        }
    }

    /// Loads all rows from the database into the change tracker as Unchanged.
    pub async fn load_all(&mut self) -> EFResult<()>
    where
        T: IFromRow
            + INavigationSetter
            + IGetKeyValues
            + IEntitySnapshot
            + crate::entity::ILazyInit,
    {
        let items = self.query().to_list().await?;
        self.clear_entries();
        for item in items {
            self.attach(item);
        }
        Ok(())
    }

    /// Marks an entity as modified.
    pub fn update(&mut self, entity: T) {
        self.entries.push(TrackedEntry {
            entity,
            state: EntityState::Modified,
            original: None,
            modified_properties: Vec::new(),
            is_upsert: false,
        });
    }

    /// Marks an entity for upsert (INSERT ... ON CONFLICT DO UPDATE).
    ///
    /// The entity is tracked in `Added` state with `is_upsert = true`. During
    /// `save_changes`, upsert entries are routed to `execute_upserts` instead
    /// of `execute_inserts`. The conflict target is the entity's primary key
    /// column(s).
    pub fn upsert(&mut self, entity: T) {
        self.entries.push(TrackedEntry {
            entity,
            state: EntityState::Added,
            original: None,
            modified_properties: Vec::new(),
            is_upsert: true,
        });
    }

    /// Compares attached entities against their original snapshots and marks changes.
    /// Populates `modified_properties` with the field names that differ from the
    /// original snapshot, enabling partial UPDATEs (SET only dirty columns).
    pub fn detect_changes(&mut self)
    where
        T: IEntitySnapshot,
    {
        for entry in &mut self.entries {
            if entry.state != EntityState::Unchanged {
                continue;
            }
            if let Some(ref original) = entry.original {
                let current = entry.entity.snapshot();
                if current == *original {
                    entry.modified_properties.clear();
                    continue;
                }
                // Collect the field names that actually differ so the
                // change executor can generate a partial UPDATE.
                let changed: Vec<String> = current
                    .iter()
                    .filter(|(k, v)| original.get(k) != Some(*v))
                    .map(|(k, _)| k.to_string())
                    .collect();
                if !changed.is_empty() {
                    entry.state = EntityState::Modified;
                    entry.modified_properties = changed;
                }
            }
        }
    }

    /// Returns tracked entities in the given state with their original snapshots,
    /// modified-property lists, and `is_upsert` flag.
    #[allow(clippy::type_complexity)]
    pub(crate) fn tracked_by_state(
        &self,
        state: EntityState,
    ) -> Vec<(&T, Option<&EntitySnapshot>, &[String], bool)> {
        self.entries
            .iter()
            .filter(|e| e.state == state)
            .map(|e| {
                (
                    &e.entity,
                    e.original.as_ref(),
                    e.modified_properties.as_slice(),
                    e.is_upsert,
                )
            })
            .collect()
    }

    /// Backfills database-generated auto-increment PKs into Added entries.
    ///
    /// Called by `save_one_set` after `execute_inserts`. `keys[i]` corresponds
    /// to the i-th non-upsert Added entry. Upsert entries (`is_upsert = true`)
    /// are skipped — their PKs are caller-managed. A key of `0` means no PK was
    /// generated (non-auto-increment or backfill skipped).
    pub(crate) fn backfill_added_keys(&mut self, keys: &[i64])
    where
        T: IGetKeyValues,
    {
        let mut idx = 0usize;
        for entry in &mut self.entries {
            if entry.state != EntityState::Added || entry.is_upsert {
                continue;
            }
            if let Some(&key) = keys.get(idx) {
                if key != 0 {
                    entry.entity.set_auto_increment_key(key);
                }
            }
            idx += 1;
        }
    }

    /// Accepts all pending changes: removes Deleted entries, transitions
    /// Added/Modified → Unchanged, and refreshes original snapshots so future
    /// `detect_changes` compares against the post-save state.
    ///
    /// Called by `save_changes()` after a successful commit. Mirrors the legacy
    /// `ChangeTracker::accept_all_changes` but operates on `DbSet.entries`.
    /// Unlike `clear_entries`, this keeps saved entities tracked (with their
    /// DB-generated PKs) so callers can read backfilled keys after save.
    pub(crate) fn accept_all_changes(&mut self)
    where
        T: IEntitySnapshot,
    {
        self.entries.retain(|e| e.state != EntityState::Deleted);
        for entry in &mut self.entries {
            if entry.state == EntityState::Added || entry.state == EntityState::Modified {
                entry.state = EntityState::Unchanged;
                entry.original = Some(entry.entity.snapshot());
                entry.modified_properties.clear();
            }
        }
    }
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

    /// Starts a query filtered by a compile-time LINQ expression tree (`linq!(�?`).
    pub fn filter<F>(&self, apply: F) -> QueryBuilder<T>
    where
        F: FnOnce(QueryBuilder<T>) -> QueryBuilder<T>,
    {
        apply(self.query())
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
    fn add(&mut self, entity: T) {
        self.entries.push(TrackedEntry {
            entity,
            state: EntityState::Added,
            original: None,
            modified_properties: Vec::new(),
            is_upsert: false,
        });
    }

    fn remove_all(&mut self) {
        for entry in &mut self.entries {
            entry.state = EntityState::Deleted;
        }
    }

    fn remove_at(&mut self, index: usize) -> EFResult<()> {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.state = EntityState::Deleted;
            Ok(())
        } else {
            Err(crate::error::EFError::not_found(
                "Entity not found at the given index".to_string(),
            ))
        }
    }

    fn attach(&mut self, entity: T) {
        let original = entity.snapshot();
        self.entries.push(TrackedEntry {
            entity,
            state: EntityState::Unchanged,
            original: Some(original),
            modified_properties: Vec::new(),
            is_upsert: false,
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
