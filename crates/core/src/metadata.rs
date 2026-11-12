//! Model metadata types for describing entities, properties, and relationships.
//!
//! These types form the metadata layer of rust-ef, analogous to EFCore's
//! `IEntityType`, `IProperty`, `INavigation`, etc.

use crate::query::BoolExpr;
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
    /// Explicit database column type (`HasColumnType`), emitted verbatim for every
    /// dialect. Overrides the Rust-type mapping (e.g. `"JSON"`,
    /// `"MEDIUMTEXT"`, `"DECIMAL(12,2)"`).
    pub column_type: Option<Cow<'static, str>>,
    /// Numeric/decimal precision (`HasPrecision(p, s)`), also used as the
    /// fractional-seconds precision for temporal types.
    pub precision: Option<u32>,
    /// Numeric/decimal scale (`HasPrecision(p, s)`).
    pub scale: Option<u32>,
    /// Whether string columns use a fixed-length type (`CHAR(n)`).
    pub is_fixed_length: bool,
    /// Column-level collation (`HasCollation`).
    pub collation: Option<Cow<'static, str>>,
    /// Column/table comment (`HasComment`).
    pub comment: Option<Cow<'static, str>>,
    /// Generated/computed column expression (`HasComputedColumnSql`).
    pub computed_sql: Option<Cow<'static, str>>,
    /// Whether a computed column is persisted (`STORED`) or virtual.
    pub computed_stored: bool,
    /// Raw SQL DEFAULT expression (`HasDefaultValue` / `HasDefaultValueSql`).
    pub default_value: Option<Cow<'static, str>>,
}

impl PropertyMeta {
    /// Returns `true` for generated/computed columns (excluded from
    /// INSERT/UPDATE and column defaults).
    pub fn is_computed(&self) -> bool {
        self.computed_sql.is_some()
    }
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
                column_type: None,
                precision: None,
                scale: None,
                is_fixed_length: false,
                collation: None,
                comment: None,
                computed_sql: None,
                computed_stored: false,
                default_value: None,
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

    pub fn column_type(mut self, name: &'static str) -> Self {
        self.meta.column_type = Some(Cow::Borrowed(name));
        self
    }

    pub fn precision(mut self, precision: u32, scale: u32) -> Self {
        self.meta.precision = Some(precision);
        self.meta.scale = Some(scale);
        self
    }

    pub fn is_fixed_length(mut self, v: bool) -> Self {
        self.meta.is_fixed_length = v;
        self
    }

    pub fn collation(mut self, name: &'static str) -> Self {
        self.meta.collation = Some(Cow::Borrowed(name));
        self
    }

    pub fn comment(mut self, text: &'static str) -> Self {
        self.meta.comment = Some(Cow::Borrowed(text));
        self
    }

    pub fn computed_sql(mut self, sql: &'static str, stored: bool) -> Self {
        self.meta.computed_sql = Some(Cow::Borrowed(sql));
        self.meta.computed_stored = stored;
        self
    }

    pub fn default_value(mut self, sql: &'static str) -> Self {
        self.meta.default_value = Some(Cow::Borrowed(sql));
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
    /// `BelongsTo<T>` — reference to a single related entity (FK on this side).
    BelongsTo,
    /// `HasOne<T>` — reference to a single related entity (FK on other side).
    HasOne,
    /// `HasMany<T>` — collection of related entities.
    HasMany,
    /// `HasMany<T, Join>` — many-to-many via join entity.
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
    /// Delete behavior for this navigation. `None` means use the default
    /// (Cascade for required FKs and M2M, Restrict for optional FKs).
    /// Set via `#[on_delete(Cascade|Restrict|SetNull|NoAction)]`.
    pub delete_behavior: Option<crate::relations::DeleteBehavior>,
}

// ---------------------------------------------------------------------------
// IndexMeta / ForeignKeyMeta / CheckMeta / TableOptions
// ---------------------------------------------------------------------------

/// One key part of an [`IndexMeta`].
///
/// Plain columns are emitted quoted; expressions are emitted verbatim (and are
/// therefore dialect-sensitive — use portable SQL such as
/// `coalesce(channel_id, '')`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexColumn {
    /// A (already resolved) database column name.
    Column(Cow<'static, str>),
    /// A raw SQL expression, e.g. `coalesce(channel_id, '')`.
    Expression(Cow<'static, str>),
}

impl IndexColumn {
    /// Returns the underlying column/expression text.
    pub fn as_str(&self) -> &str {
        match self {
            IndexColumn::Column(s) | IndexColumn::Expression(s) => s.as_ref(),
        }
    }

    /// Whether this key part is a raw expression.
    pub fn is_expression(&self) -> bool {
        matches!(self, IndexColumn::Expression(_))
    }
}

/// A declarative (possibly composite, partial, or expression) index.
///
/// Produced by [`crate::model_builder::EntityTypeBuilder::has_index`] and
/// consumed by the migration engine. A `filter` turns the index into a partial
/// index on PostgreSQL/SQLite; on MySQL a filtered *unique* index is emulated
/// with a functional (`CASE WHEN`) unique index because MySQL has no partial
/// indexes. Filtered non-unique indexes are skipped on MySQL.
#[derive(Debug, Clone, Default)]
pub struct IndexMeta {
    /// Explicit index name. `None` → derived from table + key parts.
    pub name: Option<Cow<'static, str>>,
    /// Index key parts, in order.
    pub columns: Vec<IndexColumn>,
    /// Whether this is a UNIQUE index.
    pub is_unique: bool,
    /// Optional partial-index predicate (`WHERE <expr>`).
    pub filter: Option<BoolExpr>,
    /// Included (non-key) columns (`INCLUDE (...)`); PostgreSQL only.
    pub include: Vec<Cow<'static, str>>,
}

impl IndexMeta {
    /// Derives the default index name for `table` from its key parts.
    ///
    /// Format: `ix_<table>_<part1>_<part2>...`, sanitized to identifier-safe
    /// characters and truncated to 64 characters (MySQL's identifier limit).
    pub fn derive_name(&self, table: &str) -> String {
        let mut name = format!("ix_{table}");
        for col in &self.columns {
            if col.is_expression() {
                // Expressions are not identifier-safe; append a short stable
                // hash so two expression indexes on the same table do not
                // collide.
                let mut hash: u64 = 1469598103934665603;
                for b in col.as_str().as_bytes() {
                    hash ^= *b as u64;
                    hash = hash.wrapping_mul(1099511628211);
                }
                name.push_str(&format!("_expr{hash:08x}"));
            } else {
                name.push('_');
                name.push_str(col.as_str());
            }
        }
        sanitize_index_name(&name)
    }
}

/// Sanitizes a derived index name to `[A-Za-z0-9_]`, max 64 chars.
pub(crate) fn sanitize_index_name(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.len() > 64 {
        let mut truncated: String = out.chars().take(55).collect();
        // Short, deterministic suffix to avoid collisions after truncation.
        let mut hash: u64 = 1469598103934665603;
        for b in raw.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(1099511628211);
        }
        truncated.push('_');
        truncated.push_str(&format!("{hash:08x}"));
        out = truncated;
    }
    out
}

/// A foreign key declared on an entity.
///
/// Single-column keys are usually inferred from `BelongsTo` navigations or the
/// scalar `#[foreign_key(Target)]` attribute; composite keys are declared via
/// [`crate::model_builder::EntityTypeBuilder::has_foreign_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyMeta {
    /// Explicit constraint name. `None` → derived from table + columns.
    pub name: Option<Cow<'static, str>>,
    /// Dependent columns (on this entity's table), in order.
    pub columns: Vec<Cow<'static, str>>,
    /// Principal table name.
    pub referenced_table: Cow<'static, str>,
    /// Principal columns, in order (same arity as `columns`).
    pub referenced_columns: Vec<Cow<'static, str>>,
    /// Optional `ON DELETE` SQL clause (e.g. `"CASCADE"`).
    pub on_delete: Option<Cow<'static, str>>,
}

impl ForeignKeyMeta {
    /// Derives the default constraint name for `table`.
    pub fn derive_name(&self, table: &str) -> String {
        let cols: Vec<&str> = self.columns.iter().map(|c| c.as_ref()).collect();
        sanitize_index_name(&format!(
            "fk_{table}_{}_{}",
            cols.join("_"),
            self.referenced_table
        ))
    }
}

/// A table-level `CHECK` constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckMeta {
    /// Constraint name (required by PostgreSQL/MySQL for later dropping).
    pub name: Cow<'static, str>,
    /// SQL boolean expression, e.g. `amount > 0`.
    pub expression: Cow<'static, str>,
}

/// MySQL table-level storage options (`ENGINE`, `DEFAULT CHARSET`, `COLLATE`).
///
/// Ignored by PostgreSQL and SQLite.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableOptions {
    /// Storage engine, e.g. `InnoDB`.
    pub engine: Option<Cow<'static, str>>,
    /// Default character set, e.g. `utf8mb4`.
    pub charset: Option<Cow<'static, str>>,
    /// Default collation, e.g. `utf8mb4_bin`.
    pub collate: Option<Cow<'static, str>>,
}

impl TableOptions {
    /// Returns `true` when no option is configured.
    pub fn is_empty(&self) -> bool {
        self.engine.is_none() && self.charset.is_none() && self.collate.is_none()
    }
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
    /// The name of the primary key property (simplified; multi-key via `Vec<String>` in practice).
    pub primary_keys: Vec<Cow<'static, str>>,
    /// Declarative indexes: composite, unique, partial (`filter`), or expression.
    pub indexes: Vec<IndexMeta>,
    /// Declared foreign keys (composite keys and scalar `#[foreign_key]` targets).
    pub foreign_keys: Vec<ForeignKeyMeta>,
    /// Table-level CHECK constraints.
    pub checks: Vec<CheckMeta>,
    /// MySQL table options.
    pub table_options: TableOptions,
    /// Maps to a database view (`ToView`): excluded from table DDL.
    pub is_view: bool,
    /// Keyless entity (`HasNoKey`): no primary key; read-only projections/views.
    pub is_keyless: bool,
    /// Table/entity comment (`HasComment`).
    pub comment: Option<Cow<'static, str>>,
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

    /// Returns mapped properties that can be written by INSERT/UPDATE.
    ///
    /// Computed/generated columns are readable but must never appear in
    /// INSERT/UPDATE (the database computes them).
    pub fn writable_scalar_properties(&self) -> impl Iterator<Item = &PropertyMeta> {
        self.properties
            .iter()
            .filter(|p| !p.is_not_mapped && !p.is_computed())
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
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            checks: Vec::new(),
            table_options: TableOptions::default(),
            is_view: false,
            is_keyless: false,
            comment: None,
            property_index: std::sync::OnceLock::new(),
            navigation_index: std::sync::OnceLock::new(),
        }
    }
}
