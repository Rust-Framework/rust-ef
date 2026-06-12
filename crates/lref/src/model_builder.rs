//! Model builder — Fluent API for configuring entity types.
//!
//! The `ModelBuilder` is the central configuration hub. It collects
//! `EntityTypeMeta` from all registered entity types and entity
//! configurations, and produces the final metadata model used by
//! the migration engine and query builder.
//!
//! Design note: builder structs (`EntityTypeBuilder`, `PropertyBuilder`, etc.)
//! are pure-DSL configuration types and do NOT impose `IEntityType` bounds.
//! Constraints live at the `ModelBuilder` entry points where `entity_meta()`
//! is actually called.

use crate::entity::IEntityType;
use crate::metadata::{EntityTypeMeta, PropertyMeta, PropertyMetaBuilder};
use std::any::TypeId;
use std::collections::HashMap;
use std::marker::PhantomData;

// ---------------------------------------------------------------------------
// EntityConfig — stored configuration overrides
// ---------------------------------------------------------------------------

/// Represents an override configuration for an entity type.
#[derive(Debug, Clone, Default)]
struct EntityConfig {
    table_name: Option<String>,
    property_overrides: HashMap<String, PropertyConfigOverride>,
    /// Global query filter SQL expression, e.g. `"is_deleted = false"`
    query_filter: Option<String>,
}

/// Represents a property-level configuration override.
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
/// Corresponds to EFCore's `ModelBuilder`.
pub struct ModelBuilder {
    /// Collected entity type metadata.
    entity_metas: Vec<EntityTypeMeta>,
    /// Configuration overrides by TypeId.
    configs: HashMap<TypeId, EntityConfig>,
}

impl ModelBuilder {
    pub fn new() -> Self {
        Self {
            entity_metas: Vec::new(),
            configs: HashMap::new(),
        }
    }

    /// Begins configuring an entity type and registers it in the model.
    pub fn entity<T: IEntityType>(&mut self) -> EntityTypeBuilder<T> {
        let meta = T::entity_meta();
        let type_id = meta.type_id;
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
        }
        self.configs.entry(type_id).or_default();
        EntityTypeBuilder::new()
    }

    /// Applies a single entity type configuration.
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
        let mut builder = EntityTypeBuilder::new();
        config.configure(&mut builder);

        self
    }

    /// Returns the collected entity type metadata with configurations applied.
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

    /// Returns a mutable reference to the entity type metadata.
    pub fn entity_metas_mut(&mut self) -> &mut Vec<EntityTypeMeta> {
        &mut self.entity_metas
    }

    /// Returns the metadata for a specific entity type.
    pub fn find_entity<T: IEntityType>(&self) -> Option<&EntityTypeMeta> {
        let type_id = TypeId::of::<T>();
        self.entity_metas.iter().find(|m| m.type_id == type_id)
    }

    /// Registers a global query filter for an entity type.
    /// The filter is a raw SQL WHERE clause expression applied to every query.
    ///
    /// # Example
    /// ```ignore
    /// model_builder.has_query_filter::<Blog>("is_deleted = false");
    /// ```
    pub fn has_query_filter<T: IEntityType>(&mut self, filter_sql: &str) -> &mut Self {
        let type_id = TypeId::of::<T>();
        let config = self.configs.entry(type_id).or_default();
        config.query_filter = Some(filter_sql.to_string());

        // Ensure entity is registered
        let meta = T::entity_meta();
        if !self.entity_metas.iter().any(|m| m.type_id == type_id) {
            self.entity_metas.push(meta);
        }

        self
    }

    /// Returns the global query filter for a type, if any.
    pub fn get_query_filter(&self, type_id: &TypeId) -> Option<&str> {
        self.configs
            .get(type_id)
            .and_then(|c| c.query_filter.as_deref())
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

/// Fluent builder for configuring an entity type.
///
/// Does NOT impose `IEntityType` on `T` — the constraint is on the
/// `ModelBuilder` entry points that call `T::entity_meta()`.
pub struct EntityTypeBuilder<T> {
    table_name: Option<String>,
    _phantom: PhantomData<T>,
}

impl<T> EntityTypeBuilder<T> {
    pub fn new() -> Self {
        Self {
            table_name: None,
            _phantom: PhantomData,
        }
    }

    pub fn to_table(&mut self, name: &str) -> &mut Self {
        self.table_name = Some(name.to_string());
        self
    }

    pub fn property<V: 'static>(&mut self, _accessor: fn(&T) -> &V) -> PropertyBuilder<T, V> {
        PropertyBuilder {
            _entity: PhantomData,
            _value: PhantomData,
            builder: PropertyMetaBuilder::new(
                "property",
                TypeId::of::<V>(),
                std::any::type_name::<V>(),
            ),
        }
    }

    pub fn has_key<V>(&mut self, _accessor: fn(&T) -> &V) -> &mut Self {
        self
    }

    pub fn has_many<R: IEntityType>(
        &mut self,
        _navigation: fn(&T) -> &crate::relations::HasMany<R>,
    ) -> CollectionNavigationBuilder<T, R> {
        CollectionNavigationBuilder {
            _entity: PhantomData,
            _related: PhantomData,
        }
    }

    pub fn has_one<R: IEntityType>(
        &mut self,
        _navigation: fn(&T) -> &crate::relations::HasOne<R>,
    ) -> ReferenceNavigationBuilder<T, R> {
        ReferenceNavigationBuilder {
            _entity: PhantomData,
            _related: PhantomData,
        }
    }

    pub fn has_data(&mut self, _data: &[T]) -> &mut Self {
        self
    }

    pub fn get_table_name(&self) -> Option<&str> {
        self.table_name.as_deref()
    }
}

// ---------------------------------------------------------------------------
// PropertyBuilder
// ---------------------------------------------------------------------------

pub struct PropertyBuilder<T, V> {
    _entity: PhantomData<T>,
    _value: PhantomData<V>,
    builder: PropertyMetaBuilder,
}

impl<T, V> PropertyBuilder<T, V> {
    pub fn is_required(mut self) -> Self {
        self.builder = self.builder.is_required(true);
        self
    }

    pub fn has_max_length(mut self, n: usize) -> Self {
        self.builder = self.builder.max_length(n);
        self
    }

    pub fn has_column_name(mut self, name: &'static str) -> Self {
        self.builder = self.builder.column_name(name);
        self
    }

    pub fn is_unique(mut self) -> Self {
        self.builder = self.builder.is_unique(true);
        self
    }

    pub fn has_index(mut self) -> Self {
        self.builder = self.builder.has_index(true);
        self
    }

    pub fn build(self) -> PropertyMeta {
        self.builder.build()
    }
}

// ---------------------------------------------------------------------------
// Navigation builders
// ---------------------------------------------------------------------------

pub struct CollectionNavigationBuilder<T, R> {
    _entity: PhantomData<T>,
    _related: PhantomData<R>,
}

impl<T, R> CollectionNavigationBuilder<T, R> {
    pub fn with_one(self, _inverse: fn(&R) -> &crate::relations::BelongsTo<T>) -> Self {
        self
    }
    pub fn with_many(self, _inverse: fn(&R) -> &crate::relations::HasMany<T>) -> Self {
        self
    }
    pub fn has_foreign_key<V>(self, _fk_accessor: fn(&R) -> &V) -> Self {
        self
    }
}

pub struct ReferenceNavigationBuilder<T, R> {
    _entity: PhantomData<T>,
    _related: PhantomData<R>,
}

impl<T, R> ReferenceNavigationBuilder<T, R> {
    pub fn with_one<Nav>(self, _inverse: fn(&R) -> &Nav) -> Self {
        self
    }
    pub fn has_foreign_key<V>(self, _fk_accessor: fn(&T) -> &V) -> Self {
        self
    }
}

// ---------------------------------------------------------------------------
// IEntityTypeConfiguration<T>
// ---------------------------------------------------------------------------

/// Interface for entity type configuration via the Fluent API.
///
/// The `T: IEntityType` bound is on the trait (not the implementor) because
/// `configure` operates on `EntityTypeBuilder<T>` which carries no trait
/// constraint itself — the bound signals that this configuration is intended
/// for entity types.
pub trait IEntityTypeConfiguration<T: IEntityType> {
    fn configure(&self, entity: &mut EntityTypeBuilder<T>);
}
