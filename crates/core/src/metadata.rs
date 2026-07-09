//! Model metadata types for describing entities, properties, and relationships.
//!
//! These types form the metadata layer of rust-ef, analogous to EFCore's
//! `IEntityType`, `IProperty`, `INavigation`, etc.

use std::any::TypeId;
use std::borrow::Cow;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// PropertyMeta ?describes a single column / property
// ---------------------------------------------------------------------------

/// Metadata describing a property (column) of an entity type.
/// Corresponds to EFCore's `IProperty`.
#[derive(Debug, Clone)]
pub struct PropertyMeta {
    /// The Rust field name.
    pub field_name: Cow<'static, str>,
    /// The database column name (may differ from field_name via #[column]).
    pub column_name: Cow<'static, str>,
    /// The Rust type identifier.
    pub type_id: TypeId,
    /// Human-readable type name for debugging.
    pub type_name: Cow<'static, str>,
    /// Whether this property is part of the primary key.
    pub is_primary_key: bool,
    /// Whether this property is auto-increment / identity.
    pub is_auto_increment: bool,
    /// Whether this property is backed by a database sequence (PostgreSQL).
    /// On non-PG providers, falls back to auto_increment behavior.
    pub is_sequence: bool,
    /// The sequence name when `is_sequence` is true (PostgreSQL only).
    pub sequence_name: Option<Cow<'static, str>>,
    /// Whether this property is required (NOT NULL).
    pub is_required: bool,
    /// Whether this property is a foreign key.
    pub is_foreign_key: bool,
    /// Whether this property is used for optimistic concurrency control.
    pub is_concurrency_token: bool,
    /// Maximum length for string columns (None = unbounded).
    pub max_length: Option<usize>,
    /// Whether this property has a unique index.
    pub is_unique: bool,
    /// Whether this property has a regular index.
    pub has_index: bool,
    /// Whether this property is excluded from mapping (NotMapped).
    pub is_not_mapped: bool,
}

/// Builder for configuring [`PropertyMeta`].
pub struct PropertyMetaBuilder {
    meta: PropertyMeta,
}

impl PropertyMetaBuilder {
    pub fn new(field_name: &'static str, type_id: TypeId, type_name: &'static str) -> Self {
        Self {
            meta: PropertyMeta {
                field_name: Cow::Borrowed(field_name),
                column_name: Cow::Borrowed(field_name),
                type_id,
                type_name: Cow::Borrowed(type_name),
                is_primary_key: false,
                is_auto_increment: false,
                is_sequence: false,
                sequence_name: None,
                is_required: false,
                is_foreign_key: false,
                is_concurrency_token: false,
                max_length: None,
                is_unique: false,
                has_index: false,
                is_not_mapped: false,
            },
        }
    }

    pub fn column_name(mut self, name: &'static str) -> Self {
        self.meta.column_name = Cow::Borrowed(name);
        self
    }

    pub fn is_primary_key(mut self, v: bool) -> Self {
        self.meta.is_primary_key = v;
        self
    }

    pub fn is_auto_increment(mut self, v: bool) -> Self {
        self.meta.is_auto_increment = v;
        self
    }

    pub fn is_sequence(mut self, v: bool) -> Self {
        self.meta.is_sequence = v;
        self
    }

    pub fn sequence_name(mut self, name: &'static str) -> Self {
        self.meta.sequence_name = Some(Cow::Borrowed(name));
        self
    }

    pub fn is_required(mut self, v: bool) -> Self {
        self.meta.is_required = v;
        self
    }

    pub fn is_foreign_key(mut self, v: bool) -> Self {
        self.meta.is_foreign_key = v;
        self
    }

    pub fn is_concurrency_token(mut self, v: bool) -> Self {
        self.meta.is_concurrency_token = v;
        self
    }

    pub fn max_length(mut self, n: usize) -> Self {
        self.meta.max_length = Some(n);
        self
    }

    pub fn is_unique(mut self, v: bool) -> Self {
        self.meta.is_unique = v;
        self
    }

    pub fn has_index(mut self, v: bool) -> Self {
        self.meta.has_index = v;
        self
    }

    pub fn is_not_mapped(mut self, v: bool) -> Self {
        self.meta.is_not_mapped = v;
        self
    }

    pub fn build(self) -> PropertyMeta {
        self.meta
    }
}

// ---------------------------------------------------------------------------
// NavigationMeta ?describes a navigation property (relationship)
// ---------------------------------------------------------------------------

/// Describes the type of navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationKind {
    /// BelongsTo<T> ?reference to a single related entity (FK on this side).
    BelongsTo,
    /// HasOne<T> ?reference to a single related entity (FK on other side).
    HasOne,
    /// HasMany<T> ?collection of related entities.
    HasMany,
    /// HasMany<T, Join> ?many-to-many via join entity.
    ManyToMany,
}

/// Metadata describing a navigation property (relationship).
/// Corresponds to EFCore's `INavigation`.
#[derive(Debug, Clone)]
pub struct NavigationMeta {
    /// The Rust field name of this navigation property.
    pub field_name: Cow<'static, str>,
    /// The kind of navigation (BelongsTo, HasOne, HasMany).
    pub kind: NavigationKind,
    /// The related entity's TypeId.
    pub related_type_id: TypeId,
    /// The related entity's type name.
    pub related_type_name: Cow<'static, str>,
    /// The foreign key property name on the dependent entity.
    pub foreign_key_field: Option<Cow<'static, str>>,
    /// The inverse navigation field name on the related entity.
    pub inverse_navigation: Option<Cow<'static, str>>,
    /// For many-to-many: the join entity TypeId.
    pub through_type_id: Option<TypeId>,
    /// Join table name (many-to-many).
    pub through_table: Option<Cow<'static, str>>,
    /// FK column on the join table pointing to the principal entity.
    pub through_parent_fk: Option<Cow<'static, str>>,
    /// FK column on the join table pointing to the related entity.
    pub through_related_fk: Option<Cow<'static, str>>,
    /// Column index in join-table rows for the principal FK.
    pub through_parent_fk_index: usize,
    /// Column index in join-table rows for the related FK.
    pub through_related_fk_index: usize,
    /// Related entity table name (for eager loading).
    pub related_table: Option<Cow<'static, str>>,
    /// Foreign key column on the dependent/related table.
    pub fk_column: Option<Cow<'static, str>>,
    /// Referenced key column on the principal table.
    pub referenced_key_column: Option<Cow<'static, str>>,
    /// Column index in `SELECT *` rows for the FK (HasMany grouping).
    pub fk_row_index: usize,
    /// Column index in `SELECT *` rows for the related PK (BelongsTo lookup).
    pub pk_row_index: usize,
    /// Resolves metadata for the related entity type (nested Include / ThenInclude).
    pub related_entity_meta: Option<fn() -> EntityTypeMeta>,
}

// ---------------------------------------------------------------------------
// EntityTypeMeta ?describes an entity type as a whole
// ---------------------------------------------------------------------------

/// Metadata describing an entity type.
/// Corresponds to EFCore's `IEntityType`.
#[derive(Debug, Clone)]
pub struct EntityTypeMeta {
    /// The Rust type identifier.
    pub type_id: TypeId,
    /// The Rust struct name.
    pub type_name: Cow<'static, str>,
    /// The database table name.
    pub table_name: Cow<'static, str>,
    /// Properties (columns) of this entity.
    pub properties: Vec<PropertyMeta>,
    /// Navigation properties (relationships) of this entity.
    pub navigations: Vec<NavigationMeta>,
    /// The name of the primary key property (simplified; multi-key via Vec<String> in practice).
    pub primary_keys: Vec<Cow<'static, str>>,
    /// Lazily-built `field_name -> properties index` map for O(1) lookup.
    /// Built on first `find_property` call; empty after clone (rebuilt on demand).
    /// `#[doc(hidden)]` — internal cache, do not set manually.
    #[doc(hidden)]
    pub property_index: std::sync::OnceLock<HashMap<String, usize>>,
    /// Lazily-built `field_name -> navigations index` map for O(1) lookup.
    /// Built on first `find_navigation` call; empty after clone (rebuilt on demand).
    /// `#[doc(hidden)]` — internal cache, do not set manually.
    #[doc(hidden)]
    pub navigation_index: std::sync::OnceLock<HashMap<String, usize>>,
}

impl EntityTypeMeta {
    /// Find a property by field name.
    ///
    /// Uses a lazily-built HashMap index for O(1) lookup instead of linear
    /// scan. The index is cached for the lifetime of this `EntityTypeMeta`;
    /// a cloned meta rebuilds its own index on first access.
    pub fn find_property(&self, field_name: &str) -> Option<&PropertyMeta> {
        let idx = self
            .property_index
            .get_or_init(|| {
                self.properties
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.field_name.to_string(), i))
                    .collect()
            })
            .get(field_name)
            .copied();
        idx.and_then(|i| self.properties.get(i))
    }

    /// Find a navigation by field name.
    ///
    /// Uses a lazily-built HashMap index for O(1) lookup instead of linear
    /// scan. The index is cached for the lifetime of this `EntityTypeMeta`;
    /// a cloned meta rebuilds its own index on first access.
    pub fn find_navigation(&self, field_name: &str) -> Option<&NavigationMeta> {
        let idx = self
            .navigation_index
            .get_or_init(|| {
                self.navigations
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.field_name.to_string(), i))
                    .collect()
            })
            .get(field_name)
            .copied();
        idx.and_then(|i| self.navigations.get(i))
    }

    /// Get the primary key property.
    pub fn primary_key_property(&self) -> Option<&PropertyMeta> {
        self.properties.iter().find(|p| p.is_primary_key)
    }

    /// Returns properties that are not navigation and not mapped.
    pub fn mapped_scalar_properties(&self) -> impl Iterator<Item = &PropertyMeta> {
        self.properties.iter().filter(|p| !p.is_not_mapped)
    }
}

impl Default for EntityTypeMeta {
    /// Produces an empty `EntityTypeMeta` with lazily-built index caches.
    /// `type_id` defaults to `TypeId::of::<()>()` since `TypeId` itself has no
    /// `Default` impl; callers always override it in struct literals via
    /// `..EntityTypeMeta::default()`.
    fn default() -> Self {
        Self {
            type_id: TypeId::of::<()>(),
            type_name: Cow::Borrowed(""),
            table_name: Cow::Borrowed(""),
            properties: Vec::new(),
            navigations: Vec::new(),
            primary_keys: Vec::new(),
            property_index: std::sync::OnceLock::new(),
            navigation_index: std::sync::OnceLock::new(),
        }
    }
}
