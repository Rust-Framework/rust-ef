//! Model builder  - ?Fluent API for configuring entity types.
//!
//! The `ModelBuilder` is the central configuration hub. It collects
//! `EntityTypeMeta` from all registered entity types and entity
//! configurations, and produces the final metadata model used by
//! the migration engine and query builder.

use crate::entity::{IEntitySnapshot, IEntityType};
use crate::entity_snapshot::EntitySnapshot;
use crate::metadata::EntityTypeMeta;
use crate::query::{BoolExpr, CompiledFilter};
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// EntityConfig  - ?stored configuration overrides
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub(crate) struct EntityConfig {
    pub(crate) table_name: Option<String>,
    pub(crate) primary_key_fields: Option<Vec<String>>,
    pub(crate) property_overrides: HashMap<String, PropertyConfigOverride>,
    pub(crate) query_filter: Option<BoolExpr>,
    pub(crate) seed_rows: Vec<EntitySnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PropertyConfigOverride {
    pub(crate) column_name: Option<String>,
    pub(crate) is_required: Option<bool>,
    pub(crate) max_length: Option<usize>,
    pub(crate) is_unique: Option<bool>,
    pub(crate) has_index: Option<bool>,
}

// ---------------------------------------------------------------------------
// ModelBuilder
// ---------------------------------------------------------------------------

/// Central configuration point for the EF model.
pub struct ModelBuilder {
    entity_metas: Vec<EntityTypeMeta>,
    configs: HashMap<TypeId, EntityConfig>,
    /// Cache of `build()` output. Invalidated by any mutation.
    build_cache: OnceLock<Vec<EntityTypeMeta>>,
    /// Cache of `filters_by_table()` output, shared via `Arc` so DbSets can
    /// hold a cheap clone without re-traversing configs on every query.
    filter_cache: OnceLock<Arc<HashMap<String, CompiledFilter>>>,
}

impl ModelBuilder {
    pub fn new() -> Self {
        Self {
            entity_metas: Vec::new(),
            configs: HashMap::new(),
            build_cache: OnceLock::new(),
            filter_cache: OnceLock::new(),
        }
    }

    /// Constructs a `ModelBuilder` pre-populated from a cached `BuiltMetadata`.
    ///
    /// Used by `DbContext::from_options()` to skip re-iterating `inventory::iter`
    /// and re-running `IEntityTypeConfiguration::configure()` on every request.
    /// The `build_cache` and `filter_cache` are left empty (lazy) — they will
    /// be populated on first access, same as the non-cached path.
    ///
    /// Per-instance mutations (`has_query_filter`, `entity::<T>()`, etc.) after
    /// construction only affect this `ModelBuilder` instance, not the cache.
    pub(crate) fn from_built(built: &crate::metadata_cache::BuiltMetadata) -> Self {
        Self {
            entity_metas: built.model_metas.clone(),
            configs: built.configs.clone(),
            build_cache: OnceLock::new(),
            filter_cache: OnceLock::new(),
        }
    }

    /// Read-only access to the `configs` map. Used by `MetadataCache::build()`
    /// to snapshot the configs produced by `IEntityTypeConfiguration::configure()`
    /// callbacks into the process-level cache.
    pub(crate) fn configs(&self) -> &HashMap<TypeId, EntityConfig> {
        &self.configs
    }

    /// Read-only access to the `entity_metas` vec. Used by `MetadataCache::build()`
    /// to snapshot the registered entity metas into the process-level cache.
    pub(crate) fn entity_metas_vec(&self) -> &[EntityTypeMeta] {
        &self.entity_metas
    }

    /// Drops the cached `build()` and `filters_by_table()` results.
    ///
    /// Called by every mutating method so that subsequent reads observe the
    /// new configuration. Must be `&mut self` to enforce exclusive access.
    fn invalidate_cache(&mut self) {
        self.build_cache.take();
        self.filter_cache.take();
    }

    /// Test-only hook: returns `true` if `build()` has populated its cache.
    #[doc(hidden)]
    pub fn build_cache_populated(&self) -> bool {
        self.build_cache.get().is_some()
    }

    /// Test-only hook: returns `true` if `filters_by_table()` has populated
    /// its cache.
    #[doc(hidden)]
    pub fn filter_cache_populated(&self) -> bool {
        self.filter_cache.get().is_some()
    }

    pub fn entity<T: IEntityType>(&mut self) -> EntityTypeBuilder<'_, T> {
        let meta = T::entity_meta();
        let type_id = meta.type_id;
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
            self.invalidate_cache();
        }
        self.configs.entry(type_id).or_default();
        EntityTypeBuilder::new(self, type_id)
    }

    pub fn apply_configuration<C, T>(&mut self) -> &mut Self
    where
        C: IEntityTypeConfiguration<T> + Default + Send + Sync + 'static,
        T: IEntityType,
    {
        let meta = T::entity_meta();
        let type_id = meta.type_id;
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
            self.invalidate_cache();
        }

        let config = C::default();
        let mut builder = EntityTypeBuilder::new(self, type_id);
        config.configure(&mut builder);

        self
    }

    pub fn build(&self) -> Vec<EntityTypeMeta> {
        self.build_cache
            .get_or_init(|| {
                self.entity_metas
                    .iter()
                    .map(|meta| self.apply_config_to_meta(meta))
                    .collect()
            })
            .clone()
    }

    fn apply_config_to_meta(&self, meta: &EntityTypeMeta) -> EntityTypeMeta {
        let config = match self.configs.get(&meta.type_id) {
            Some(c) => c,
            None => return meta.clone(),
        };

        let mut result = meta.clone();
        if let Some(ref table) = config.table_name {
            result.table_name = std::borrow::Cow::Owned(table.clone());
        }

        if let Some(ref pk_fields) = config.primary_key_fields {
            for prop in &mut result.properties {
                prop.is_primary_key = pk_fields.iter().any(|f| f == prop.field_name.as_ref());
            }
        }

        for prop in &mut result.properties {
            if let Some(override_cfg) = config.property_overrides.get(prop.field_name.as_ref()) {
                if let Some(ref col) = override_cfg.column_name {
                    prop.column_name = std::borrow::Cow::Owned(col.clone());
                }
                if let Some(required) = override_cfg.is_required {
                    prop.is_required = required;
                }
                if let Some(max_len) = override_cfg.max_length {
                    prop.max_length = Some(max_len);
                }
                if let Some(unique) = override_cfg.is_unique {
                    prop.is_unique = unique;
                }
                if let Some(index) = override_cfg.has_index {
                    prop.has_index = index;
                }
            }
        }

        result
    }

    /// Escape hatch for direct mutation of `entity_metas`.
    ///
    /// **Cache caveat**: this returns a `&mut Vec`, so mutations performed
    /// through it cannot auto-invalidate `build()` / `filters_by_table()`.
    /// Callers that mutate via this handle must drop the borrow and perform
    /// any subsequent Fluent API mutation (or never read the caches again
    /// in this context). Prefer `register_entity_meta()` / `entity::<T>()`
    /// for cache-safe registration.
    pub fn entity_metas_mut(&mut self) -> &mut Vec<EntityTypeMeta> {
        // Note: cannot call invalidate_cache() here because the returned
        // &mut Vec would conflict. Documented as a manual-invalidation path.
        &mut self.entity_metas
    }

    pub fn find_entity<T: IEntityType>(&self) -> Option<&EntityTypeMeta> {
        let type_id = TypeId::of::<T>();
        self.entity_metas.iter().find(|m| m.type_id == type_id)
    }

    /// Returns `true` if an entity with the given `type_id` is already
    /// registered in STORE B.
    pub fn has_entity(&self, type_id: TypeId) -> bool {
        self.entity_metas.iter().any(|m| m.type_id == type_id)
    }

    /// Registers an entity meta directly, without going through
    /// `entity::<T>()`. Used by `DbContext::discover_entities()` to populate
    /// STORE B from `inventory::iter::<EntityRegistration>()`.
    ///
    /// Ensures a corresponding `EntityConfig` entry exists so that subsequent
    /// Fluent API calls (`to_table`, `property_named`, etc.) have somewhere
    /// to write their overrides.
    pub fn register_entity_meta(&mut self, meta: EntityTypeMeta) {
        let type_id = meta.type_id;
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
            self.invalidate_cache();
        }
        self.configs.entry(type_id).or_default();
    }

    /// Registers a global query filter for entity type `T`.
    ///
    /// Accepts a `BoolExpr` produced by `linq!(filter |b: T| ...)`.
    pub fn has_query_filter<T: IEntityType>(&mut self, filter: BoolExpr) -> &mut Self {
        let type_id = TypeId::of::<T>();
        let config = self.configs.entry(type_id).or_default();
        config.query_filter = Some(filter);

        let meta = T::entity_meta();
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
        }

        self.invalidate_cache();
        self
    }

    pub fn get_query_filter(&self, type_id: &TypeId) -> Option<&BoolExpr> {
        self.configs
            .get(type_id)
            .and_then(|c| c.query_filter.as_ref())
    }

    /// Collects all registered query filters keyed by compile-time table name
    /// (i.e. `EntityTypeMeta.table_name` from `#[table("...")]`).
    ///
    /// This must match the `related_table` stored in navigation metadata, which
    /// is also the compile-time name — NOT the Fluent API override. If a Fluent
    /// `to_table()` override renames the table, the navigation SQL and the
    /// filter lookup both use the compile-time name, staying consistent.
    ///
    /// Each filter is wrapped in a `CompiledFilter` that pre-collects parameter
    /// values at registration time, avoiding redundant `collect_bool_expr_values`
    /// traversals on every query. The SQL fragment is still compiled per query
    /// (dialect-specific placeholders depend on the provider and query context).
    pub fn filters_by_table(&self) -> Arc<HashMap<String, CompiledFilter>> {
        self.filter_cache
            .get_or_init(|| {
                let mut map = HashMap::new();
                for meta in &self.entity_metas {
                    if let Some(config) = self.configs.get(&meta.type_id) {
                        if let Some(filter) = &config.query_filter {
                            map.insert(
                                meta.table_name.to_string(),
                                CompiledFilter::new(filter.clone()),
                            );
                        }
                    }
                }
                Arc::new(map)
            })
            .clone()
    }

    /// Returns seed rows configured via `EntityTypeBuilder::has_data`.
    pub fn seed_rows_for(&self, type_id: &TypeId) -> &[EntitySnapshot] {
        self.configs
            .get(type_id)
            .map(|c| c.seed_rows.as_slice())
            .unwrap_or(&[])
    }
}

impl Default for ModelBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// EntityTypeBuilder<T>
// ---------------------------------------------------------------------------

pub struct EntityTypeBuilder<'a, T> {
    model: &'a mut ModelBuilder,
    type_id: TypeId,
    _phantom: PhantomData<T>,
}

impl<'a, T> EntityTypeBuilder<'a, T> {
    pub fn new(model: &'a mut ModelBuilder, type_id: TypeId) -> Self {
        Self {
            model,
            type_id,
            _phantom: PhantomData,
        }
    }

    pub fn to_table(&mut self, name: &str) -> &mut Self {
        if let Some(config) = self.model.configs.get_mut(&self.type_id) {
            config.table_name = Some(name.to_string());
        }
        self.model.invalidate_cache();
        self
    }

    pub fn property_named<'b>(&'b mut self, field_name: &'static str) -> PropertyBuilder<'b, T> {
        PropertyBuilder {
            model: self.model,
            type_id: self.type_id,
            field_name,
            _entity: PhantomData,
            _value: PhantomData,
        }
    }

    pub fn has_key_named(&mut self, field_name: &str) -> &mut Self {
        if let Some(config) = self.model.configs.get_mut(&self.type_id) {
            config.primary_key_fields = Some(vec![field_name.to_string()]);
        }
        self.model.invalidate_cache();
        self
    }

    pub fn has_keys(&mut self, field_names: &[&str]) -> &mut Self {
        if let Some(config) = self.model.configs.get_mut(&self.type_id) {
            config.primary_key_fields = Some(field_names.iter().map(|s| s.to_string()).collect());
        }
        self.model.invalidate_cache();
        self
    }

    /// Configures the primary key from column constants.
    ///
    /// `#[doc(hidden)]` — called by `linq!(key |b: T| b.id)` expansion (Form C).
    #[doc(hidden)]
    pub fn has_key(&mut self, columns: &'static [&'static str]) -> &mut Self {
        if let Some(config) = self.model.configs.get_mut(&self.type_id) {
            config.primary_key_fields = Some(columns.iter().map(|s| s.to_string()).collect());
        }
        self.model.invalidate_cache();
        self
    }

    /// Configures a multi-column index from column constants.
    ///
    /// `#[doc(hidden)]` — called by `linq!(index |b: T| (b.author_id, b.created_at))`
    /// expansion (Form C). Currently marks the first column as indexed; full
    /// composite index support will be added in a future iteration.
    #[doc(hidden)]
    pub fn has_index(&mut self, columns: &'static [&'static str]) -> &mut Self {
        if let Some(config) = self.model.configs.get_mut(&self.type_id) {
            // Mark each column as indexed (single-column indexes).
            for col in columns {
                let override_cfg = config
                    .property_overrides
                    .entry(col.to_string())
                    .or_default();
                override_cfg.has_index = Some(true);
            }
        }
        self.model.invalidate_cache();
        self
    }

    /// Seeds initial data (applied on `DbContext::ensure_created`).
    pub fn has_data(&mut self, data: &[T]) -> &mut Self
    where
        T: IEntitySnapshot,
    {
        if let Some(config) = self.model.configs.get_mut(&self.type_id) {
            config.seed_rows = data.iter().map(|e| e.snapshot()).collect();
        }
        self.model.invalidate_cache();
        self
    }
}

// ---------------------------------------------------------------------------
// PropertyBuilder
// ---------------------------------------------------------------------------

pub struct PropertyBuilder<'a, T, V = ()> {
    model: &'a mut ModelBuilder,
    type_id: TypeId,
    field_name: &'static str,
    _entity: PhantomData<T>,
    _value: PhantomData<V>,
}

impl<'a, T, V> PropertyBuilder<'a, T, V> {
    fn override_entry(&mut self) -> &mut PropertyConfigOverride {
        // Invalidate caches before mutating so a subsequent `build()` /
        // `filters_by_table()` reflects the new override.
        self.model.invalidate_cache();
        self.model
            .configs
            .entry(self.type_id)
            .or_default()
            .property_overrides
            .entry(self.field_name.to_string())
            .or_default()
    }

    pub fn is_required(mut self) -> Self {
        self.override_entry().is_required = Some(true);
        self
    }

    pub fn has_max_length(mut self, n: usize) -> Self {
        self.override_entry().max_length = Some(n);
        self
    }

    pub fn has_column_name(mut self, name: &'static str) -> Self {
        self.override_entry().column_name = Some(name.to_string());
        self
    }

    pub fn is_unique(mut self) -> Self {
        self.override_entry().is_unique = Some(true);
        self
    }

    pub fn has_index(mut self) -> Self {
        self.override_entry().has_index = Some(true);
        self
    }
}

// ---------------------------------------------------------------------------
// IEntityTypeConfiguration<T>
// ---------------------------------------------------------------------------

pub trait IEntityTypeConfiguration<T: IEntityType> {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, T>);
}
