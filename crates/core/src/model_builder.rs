//! Model builder  - ?Fluent API for configuring entity types.
//!
//! The `ModelBuilder` is the central configuration hub. It collects
//! `EntityTypeMeta` from all registered entity types and entity
//! configurations, and produces the final metadata model used by
//! the migration engine and query builder.

use crate::entity::{IEntitySnapshot, IEntityType};
use crate::metadata::EntityTypeMeta;
use crate::provider::DbValue;
use crate::query::BoolExpr;
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;

// ---------------------------------------------------------------------------
// EntityConfig  - ?stored configuration overrides
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct EntityConfig {
    table_name: Option<String>,
    primary_key_fields: Option<Vec<String>>,
    property_overrides: HashMap<String, PropertyConfigOverride>,
    query_filter: Option<BoolExpr>,
    seed_rows: Vec<HashMap<String, DbValue>>,
}

#[derive(Debug, Clone, Default)]
struct PropertyConfigOverride {
    column_name: Option<String>,
    is_required: Option<bool>,
    max_length: Option<usize>,
    is_unique: Option<bool>,
    has_index: Option<bool>,
}

// ---------------------------------------------------------------------------
// ModelBuilder
// ---------------------------------------------------------------------------

/// Central configuration point for the EF model.
pub struct ModelBuilder {
    entity_metas: Vec<EntityTypeMeta>,
    configs: HashMap<TypeId, EntityConfig>,
}

impl ModelBuilder {
    pub fn new() -> Self {
        Self {
            entity_metas: Vec::new(),
            configs: HashMap::new(),
        }
    }

    pub fn entity<T: IEntityType>(&mut self) -> EntityTypeBuilder<'_, T> {
        let meta = T::entity_meta();
        let type_id = meta.type_id;
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
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
        }

        let config = C::default();
        let mut builder = EntityTypeBuilder::new(self, type_id);
        config.configure(&mut builder);

        self
    }

    pub fn build(&self) -> Vec<EntityTypeMeta> {
        self.entity_metas
            .iter()
            .map(|meta| self.apply_config_to_meta(meta))
            .collect()
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

    pub fn entity_metas_mut(&mut self) -> &mut Vec<EntityTypeMeta> {
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

        self
    }

    pub fn get_query_filter(&self, type_id: &TypeId) -> Option<&BoolExpr> {
        self.configs
            .get(type_id)
            .and_then(|c| c.query_filter.as_ref())
    }

    /// Collects all registered query filters keyed by (effective) table name.
    /// Used by NavigationLoader to apply tenant isolation to secondary queries.
    pub fn filters_by_table(&self) -> HashMap<String, BoolExpr> {
        let mut map = HashMap::new();
        for meta in &self.entity_metas {
            if let Some(config) = self.configs.get(&meta.type_id) {
                if let Some(filter) = &config.query_filter {
                    let table_name = config
                        .table_name
                        .clone()
                        .unwrap_or_else(|| meta.table_name.to_string());
                    map.insert(table_name, filter.clone());
                }
            }
        }
        map
    }

    /// Returns seed rows configured via `EntityTypeBuilder::has_data`.
    pub fn seed_rows_for(&self, type_id: &TypeId) -> &[HashMap<String, DbValue>] {
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
        self
    }

    pub fn has_keys(&mut self, field_names: &[&str]) -> &mut Self {
        if let Some(config) = self.model.configs.get_mut(&self.type_id) {
            config.primary_key_fields = Some(field_names.iter().map(|s| s.to_string()).collect());
        }
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
