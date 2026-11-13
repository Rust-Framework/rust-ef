//! Model builder  - ?Fluent API for configuring entity types.
//!
//! The `ModelBuilder` is the central configuration hub. It collects
//! `EntityTypeMeta` from all registered entity types and entity
//! configurations, and produces the final metadata model used by
//! the migration engine and query builder.

use crate::entity::{IEntitySnapshot, IEntityType};
use crate::entity_snapshot::EntitySnapshot;
use crate::metadata::{
    CheckMeta, EntityTypeMeta, ForeignKeyMeta, IndexColumn, IndexMeta, TableOptions,
};
use crate::query::{BoolExpr, CompiledFilter, Predicate};
use crate::relations::DeleteBehavior;
use std::any::TypeId;
use std::borrow::Cow;
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
    /// Schema/database prefix (`ToTable(name, schema)`).
    pub(crate) table_schema: Option<String>,
    /// Maps to an existing view (`ToView`) — excluded from table DDL.
    pub(crate) is_view: bool,
    /// Keyless entity (`HasNoKey`).
    pub(crate) is_keyless: bool,
    /// Table comment (`HasComment`).
    pub(crate) table_comment: Option<String>,
    /// Fields ignored via `Ignore()`.
    pub(crate) ignored_fields: Vec<String>,
    pub(crate) primary_key_fields: Option<Vec<String>>,
    pub(crate) property_overrides: HashMap<String, PropertyConfigOverride>,
    pub(crate) query_filter: Option<BoolExpr>,
    pub(crate) seed_rows: Vec<EntitySnapshot>,
    /// Declarative indexes (composite / partial / expression).
    pub(crate) indexes: Vec<IndexMeta>,
    /// Alternate keys (unique constraints), resolved to table-qualified index
    /// names at model-build time.
    pub(crate) alternate_keys: Vec<Vec<String>>,
    /// Declared foreign keys (composite; single-column scalar FKs come from the
    /// derive macro).
    pub(crate) foreign_keys: Vec<ForeignKeyMeta>,
    /// Table-level CHECK constraints declared via `has_check`.
    pub(crate) checks: Vec<CheckMeta>,
    /// MySQL table options.
    pub(crate) table_options: TableOptions,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PropertyConfigOverride {
    pub(crate) column_name: Option<String>,
    pub(crate) is_required: Option<bool>,
    pub(crate) max_length: Option<usize>,
    pub(crate) is_unique: Option<bool>,
    pub(crate) has_index: Option<bool>,
    /// Raw SQL DEFAULT expression.
    pub(crate) default_value: Option<String>,
    /// Raw SQL CHECK expression (constraint name derived from table+column).
    pub(crate) check: Option<String>,
    /// Explicit database column type (`HasColumnType`).
    pub(crate) column_type: Option<String>,
    /// Numeric/decimal/temporal precision and scale.
    pub(crate) precision: Option<u32>,
    pub(crate) scale: Option<u32>,
    /// Fixed-length string (`CHAR(n)`).
    pub(crate) is_fixed_length: Option<bool>,
    /// Column-level collation.
    pub(crate) collation: Option<String>,
    /// Column comment.
    pub(crate) comment: Option<String>,
    /// Generated/computed column expression + storage mode.
    pub(crate) computed_sql: Option<String>,
    pub(crate) computed_stored: bool,
    /// Optimistic concurrency token.
    pub(crate) is_concurrency_token: Option<bool>,
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
        // Schema qualification: `schema.table`. Dotted identifiers are quoted
        // per part by the dialects/providers.
        if let Some(ref schema) = config.table_schema {
            let base = result.table_name.to_string();
            // A derive-level `#[table(..., schema = "...")]` already
            // qualified the table.
            if !base.contains('.') {
                result.table_name = std::borrow::Cow::Owned(format!("{schema}.{base}"));
            }
        }
        if config.is_view {
            result.is_view = true;
        }
        if config.is_keyless {
            result.is_keyless = true;
        }
        if let Some(ref comment) = config.table_comment {
            result.comment = Some(std::borrow::Cow::Owned(comment.clone()));
        }

        if let Some(ref pk_fields) = config.primary_key_fields {
            for prop in &mut result.properties {
                prop.is_primary_key = pk_fields.iter().any(|f| f == prop.field_name.as_ref());
            }
            result.primary_keys = pk_fields
                .iter()
                .map(|f| std::borrow::Cow::Owned(f.clone()))
                .collect();
        }

        for prop in &mut result.properties {
            if config
                .ignored_fields
                .iter()
                .any(|f| f == prop.field_name.as_ref())
            {
                prop.is_not_mapped = true;
            }
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
                if let Some(ref column_type) = override_cfg.column_type {
                    prop.column_type = Some(std::borrow::Cow::Owned(column_type.clone()));
                }
                if let Some(precision) = override_cfg.precision {
                    prop.precision = Some(precision);
                }
                if let Some(scale) = override_cfg.scale {
                    prop.scale = Some(scale);
                }
                if let Some(fixed) = override_cfg.is_fixed_length {
                    prop.is_fixed_length = fixed;
                }
                if let Some(ref collation) = override_cfg.collation {
                    prop.collation = Some(std::borrow::Cow::Owned(collation.clone()));
                }
                if let Some(ref comment) = override_cfg.comment {
                    prop.comment = Some(std::borrow::Cow::Owned(comment.clone()));
                }
                if let Some(ref computed_sql) = override_cfg.computed_sql {
                    prop.computed_sql = Some(std::borrow::Cow::Owned(computed_sql.clone()));
                    prop.computed_stored = override_cfg.computed_stored;
                }
                if let Some(ref default_sql) = override_cfg.default_value {
                    prop.default_value = Some(std::borrow::Cow::Owned(default_sql.clone()));
                }
                if let Some(token) = override_cfg.is_concurrency_token {
                    prop.is_concurrency_token = token;
                }
            }
            // Generated columns are computed by the database: a DEFAULT and
            // NOT NULL constraint are not applicable.
            if prop.computed_sql.is_some() {
                prop.default_value = None;
                prop.is_required = false;
            }
        }

        // Property-level CHECK constraints, named after table + column.
        for prop in &result.properties {
            if let Some(override_cfg) = config.property_overrides.get(prop.field_name.as_ref()) {
                if let Some(ref check_sql) = override_cfg.check {
                    result.checks.push(CheckMeta {
                        name: Cow::Owned(format!(
                            "CK_{}_{}",
                            result.table_name.replace('.', "_"),
                            prop.column_name
                        )),
                        expression: Cow::Owned(check_sql.clone()),
                    });
                }
            }
        }

        // Alternate keys become table-qualified unique indexes.
        for cols in &config.alternate_keys {
            let name = crate::metadata::sanitize_index_name(&format!(
                "ak_{}_{}",
                result.table_name,
                cols.join("_")
            ));
            result.indexes.push(IndexMeta {
                name: Some(Cow::Owned(name)),
                columns: cols
                    .iter()
                    .map(|c| IndexColumn::Column(Cow::Owned(c.clone())))
                    .collect(),
                is_unique: true,
                filter: None,
                include: Vec::new(),
            });
        }

        // Declarative indexes / foreign keys / table-level checks / options.
        result.indexes.extend(config.indexes.iter().cloned());
        result
            .foreign_keys
            .extend(config.foreign_keys.iter().cloned());
        result.checks.extend(config.checks.iter().cloned());
        if !config.table_options.is_empty() {
            result.table_options = config.table_options.clone();
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
    /// Accepts what `linq!(|b: T| ...)` returns (`Predicate<T>`, which also
    /// infers `T`) as well as a bare `BoolExpr`.
    pub fn has_query_filter<T: IEntityType>(
        &mut self,
        filter: impl Into<Predicate<T>>,
    ) -> &mut Self {
        let filter = filter.into().into_expr();
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
                // Keyed by the compile-time table name (same identity used by
                // navigation metadata / lazy loading). Fluent `to_table` /
                // `to_schema` overrides do not change this key — see the
                // doc comment above.
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
        let config = self.model.configs.entry(self.type_id).or_default();
        config.table_name = Some(name.to_string());
        config.is_view = false;
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
        let config = self.model.configs.entry(self.type_id).or_default();
        config.primary_key_fields = Some(vec![field_name.to_string()]);
        config.is_keyless = false;
        self.model.invalidate_cache();
        self
    }

    pub fn has_keys(&mut self, field_names: &[&str]) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.primary_key_fields = Some(field_names.iter().map(|s| s.to_string()).collect());
        config.is_keyless = false;
        self.model.invalidate_cache();
        self
    }

    /// Qualifies the table with a schema/database prefix (`ToTable(name, schema)`).
    ///
    /// Generates `schema.table` (quoted per part), e.g. PostgreSQL `"audit"."logs"`.
    pub fn to_schema(&mut self, schema: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.table_schema = Some(schema.to_string());
        self.model.invalidate_cache();
        self
    }

    /// Maps this entity to an existing database view (`ToView`).
    ///
    /// View entities are **excluded from table DDL** (no CREATE/DROP/ALTER);
    /// the view itself is managed outside rust-ef. The entity is read-only in
    /// practice (`save_changes` should not be used for it).
    pub fn to_view(&mut self, name: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.table_name = Some(name.to_string());
        config.is_view = true;
        self.model.invalidate_cache();
        self
    }

    /// Marks the entity as keyless (`HasNoKey`): no primary key is emitted.
    ///
    /// Useful for projections/views. `find`/`remove` require a key and must not
    /// be used on keyless entities.
    pub fn has_no_key(&mut self) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.primary_key_fields = Some(Vec::new());
        config.is_keyless = true;
        self.model.invalidate_cache();
        self
    }

    /// Sets the table/entity comment (`HasComment`).
    pub fn has_comment(&mut self, comment: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.table_comment = Some(comment.to_string());
        self.model.invalidate_cache();
        self
    }

    /// Ignores a scalar field (`Ignore()`), excluding it from mapping/DDL.
    pub fn ignore(&mut self, field_name: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        if !config.ignored_fields.iter().any(|f| f == field_name) {
            config.ignored_fields.push(field_name.to_string());
        }
        self.model.invalidate_cache();
        self
    }

    /// Declares an alternate key (`HasAlternateKey`): a unique constraint that
    /// can be referenced by foreign keys.
    ///
    /// Implemented as a unique index named `ak_<table>_<cols>` (equivalent for
    /// PostgreSQL/SQLite/MySQL foreign-key targets).
    pub fn has_alternate_key(&mut self, columns: &'static [&'static str]) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config
            .alternate_keys
            .push(columns.iter().map(|c| c.to_string()).collect());
        self.model.invalidate_cache();
        self
    }

    /// Configures the primary key from column constants.
    ///
    /// `#[doc(hidden)]` — called by `linq!(key |b: T| b.id)` expansion.
    #[doc(hidden)]
    pub fn has_key(&mut self, columns: &'static [&'static str]) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.primary_key_fields = Some(columns.iter().map(|s| s.to_string()).collect());
        config.is_keyless = false;
        self.model.invalidate_cache();
        self
    }

    /// Declares a (possibly composite) index from column constants.
    ///
    /// Use `linq!(index |b: T| (b.author_id, b.created_at))` to build the
    /// column array type-safely. The returned [`IndexBuilder`] configures
    /// uniqueness, an explicit name, or a partial-index predicate:
    ///
    /// ```ignore
    /// entity.has_index(&[Order::COLUMN_TENANT_ID, Order::COLUMN_REQUEST_KEY])
    ///       .is_unique();
    /// entity.has_index(&[Invite::COLUMN_TENANT_ID, Invite::COLUMN_CODE])
    ///       .is_unique()
    ///       .has_filter(linq!(|i: Invite| i.status == "PENDING"));
    /// ```
    pub fn has_index(&mut self, columns: &'static [&'static str]) -> IndexBuilder<'_, T> {
        let parts = columns
            .iter()
            .map(|c| IndexColumn::Column(Cow::Borrowed(*c)))
            .collect();
        self.push_index(parts)
    }

    /// Declares an index over raw SQL expressions, e.g.
    /// `has_index_expression(&["coalesce(channel_id, '')"])`.
    ///
    /// Expressions are emitted verbatim, so write portable SQL (lower-case
    /// identifiers work unquoted on PostgreSQL, SQLite and MySQL).
    pub fn has_index_expression(
        &mut self,
        expressions: &'static [&'static str],
    ) -> IndexBuilder<'_, T> {
        let parts = expressions
            .iter()
            .map(|e| IndexColumn::Expression(Cow::Borrowed(*e)))
            .collect();
        self.push_index(parts)
    }

    fn push_index(&mut self, columns: Vec<IndexColumn>) -> IndexBuilder<'_, T> {
        let config = self.model.configs.entry(self.type_id).or_default();
        let position = config.indexes.len();
        config.indexes.push(IndexMeta {
            name: None,
            columns,
            is_unique: false,
            filter: None,
            include: Vec::new(),
        });
        self.model.invalidate_cache();
        IndexBuilder {
            model: self.model,
            type_id: self.type_id,
            position,
            _entity: PhantomData,
        }
    }

    /// Declares a (possibly composite) foreign key.
    ///
    /// Single-column keys are usually inferred from `BelongsTo` navigations or
    /// `#[foreign_key(Target)]`; use this for composite keys such as
    /// `(tenant_id, order_id) → order_record(tenant_id, id)`:
    ///
    /// ```ignore
    /// entity.has_foreign_key(
    ///     &[Voucher::COLUMN_TENANT_ID, Voucher::COLUMN_ORDER_ID],
    ///     OrderRecord::TABLE,
    ///     &[OrderRecord::COLUMN_TENANT_ID, OrderRecord::COLUMN_ID],
    /// );
    /// ```
    pub fn has_foreign_key(
        &mut self,
        columns: &'static [&'static str],
        referenced_table: &'static str,
        referenced_columns: &'static [&'static str],
    ) -> ForeignKeyBuilder<'_, T> {
        let config = self.model.configs.entry(self.type_id).or_default();
        let position = config.foreign_keys.len();
        config.foreign_keys.push(ForeignKeyMeta {
            name: None,
            columns: columns.iter().map(|c| Cow::Borrowed(*c)).collect(),
            referenced_table: Cow::Borrowed(referenced_table),
            referenced_columns: referenced_columns
                .iter()
                .map(|c| Cow::Borrowed(*c))
                .collect(),
            on_delete: None,
        });
        self.model.invalidate_cache();
        ForeignKeyBuilder {
            model: self.model,
            type_id: self.type_id,
            position,
            _entity: PhantomData,
        }
    }

    /// Declares a table-level `CHECK` constraint.
    pub fn has_check(&mut self, name: &str, expression: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.checks.push(CheckMeta {
            name: Cow::Owned(name.to_string()),
            expression: Cow::Owned(expression.to_string()),
        });
        self.model.invalidate_cache();
        self
    }

    /// Sets the MySQL storage engine (e.g. `InnoDB`). Ignored on other dialects.
    pub fn engine(&mut self, name: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.table_options.engine = Some(Cow::Owned(name.to_string()));
        self.model.invalidate_cache();
        self
    }

    /// Sets the MySQL default character set (e.g. `utf8mb4`).
    pub fn charset(&mut self, name: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.table_options.charset = Some(Cow::Owned(name.to_string()));
        self.model.invalidate_cache();
        self
    }

    /// Sets the MySQL default collation (e.g. `utf8mb4_bin`).
    pub fn collate(&mut self, name: &str) -> &mut Self {
        let config = self.model.configs.entry(self.type_id).or_default();
        config.table_options.collate = Some(Cow::Owned(name.to_string()));
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
// IndexBuilder / ForeignKeyBuilder
// ---------------------------------------------------------------------------

/// Configures an index declared by [`EntityTypeBuilder::has_index`] or
/// [`EntityTypeBuilder::has_index_expression`].
pub struct IndexBuilder<'a, T> {
    model: &'a mut ModelBuilder,
    type_id: TypeId,
    position: usize,
    _entity: PhantomData<T>,
}

impl<T> IndexBuilder<'_, T> {
    fn entry(&mut self) -> &mut IndexMeta {
        self.model.invalidate_cache();
        &mut self
            .model
            .configs
            .get_mut(&self.type_id)
            .expect("index config registered")
            .indexes[self.position]
    }

    /// Marks the index UNIQUE.
    pub fn is_unique(&mut self) -> &mut Self {
        self.entry().is_unique = true;
        self
    }

    /// Sets an explicit index name (otherwise derived from table + columns).
    pub fn has_name(&mut self, name: &'static str) -> &mut Self {
        self.entry().name = Some(Cow::Borrowed(name));
        self
    }

    /// Turns the index into a partial index with the given predicate.
    ///
    /// Applied as `... WHERE <predicate>` on PostgreSQL/SQLite. MySQL has no
    /// partial indexes: a filtered **unique** index is emulated with a
    /// functional `CASE WHEN` unique index, and a filtered non-unique index is
    /// skipped with a warning comment.
    pub fn has_filter(&mut self, filter: impl Into<BoolExpr>) -> &mut Self {
        self.entry().filter = Some(filter.into());
        self
    }

    /// Adds non-key included columns (`INCLUDE (...)`).
    ///
    /// Supported by PostgreSQL; MySQL/SQLite emit a warning comment and create
    /// the index without the include list.
    pub fn include(&mut self, columns: &'static [&'static str]) -> &mut Self {
        let entry = self.entry();
        entry
            .include
            .extend(columns.iter().map(|c| Cow::Borrowed(*c)));
        self
    }
}

/// Configures a foreign key declared by
/// [`EntityTypeBuilder::has_foreign_key`].
pub struct ForeignKeyBuilder<'a, T> {
    model: &'a mut ModelBuilder,
    type_id: TypeId,
    position: usize,
    _entity: PhantomData<T>,
}

impl<T> ForeignKeyBuilder<'_, T> {
    fn entry(&mut self) -> &mut ForeignKeyMeta {
        self.model.invalidate_cache();
        &mut self
            .model
            .configs
            .get_mut(&self.type_id)
            .expect("foreign key config registered")
            .foreign_keys[self.position]
    }

    /// Sets an explicit constraint name (otherwise derived from table+columns).
    pub fn has_name(&mut self, name: &'static str) -> &mut Self {
        self.entry().name = Some(Cow::Borrowed(name));
        self
    }

    /// Sets the `ON DELETE` behavior.
    pub fn on_delete(&mut self, behavior: DeleteBehavior) -> &mut Self {
        self.entry().on_delete = Some(Cow::Borrowed(behavior.to_sql_clause()));
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

    /// Marks the column nullable (opposite of [`Self::is_required`]).
    pub fn is_optional(mut self) -> Self {
        self.override_entry().is_required = Some(false);
        self
    }

    /// Alias of [`Self::is_optional`] (EF Core's `IsNullable` spelling).
    ///
    /// Explicitly overrides the `Option<T>`-based requiredness inference: a
    /// non-`Option` field marked with this method generates a nullable column.
    pub fn is_nullable(mut self) -> Self {
        self.override_entry().is_required = Some(false);
        self
    }

    /// Overrides the database column type verbatim for every dialect
    /// (`HasColumnType`), e.g. `"JSON"`, `"MEDIUMTEXT"`, `"DECIMAL(12,2)"`.
    pub fn has_column_type(mut self, sql_type: &str) -> Self {
        self.override_entry().column_type = Some(sql_type.to_string());
        self
    }

    /// Sets decimal/numeric precision and scale (`HasPrecision(p, s)`).
    ///
    /// For temporal types the precision is emitted as the fractional-seconds
    /// precision (`DATETIME(p)` / `TIMESTAMP(p)`).
    pub fn has_precision(mut self, precision: u32, scale: u32) -> Self {
        let entry = self.override_entry();
        entry.precision = Some(precision);
        entry.scale = Some(scale);
        self
    }

    /// Uses a fixed-length string type (`CHAR(n)`), `n = max_length`.
    pub fn is_fixed_length(mut self) -> Self {
        self.override_entry().is_fixed_length = Some(true);
        self
    }

    /// Sets a column-level collation (`HasCollation`), emitted as
    /// `COLLATE <name>` (include quotes yourself for quoted names).
    pub fn has_collation(mut self, name: &str) -> Self {
        self.override_entry().collation = Some(name.to_string());
        self
    }

    /// Sets a column comment (`HasComment`).
    pub fn has_comment(mut self, text: &str) -> Self {
        self.override_entry().comment = Some(text.to_string());
        self
    }

    /// Declares a generated/computed column (`HasComputedColumnSql`).
    ///
    /// `sql` is a raw SQL expression (e.g. `unit_price * quantity`). Computed
    /// columns are excluded from INSERT/UPDATE. Use [`Self::is_stored`] to
    /// persist the value (required by PostgreSQL).
    pub fn has_computed_column_sql(mut self, sql: &str) -> Self {
        let entry = self.override_entry();
        entry.computed_sql = Some(sql.to_string());
        self
    }

    /// Persists a computed column (`STORED`); no-op for non-computed columns.
    pub fn is_stored(mut self) -> Self {
        self.override_entry().computed_stored = true;
        self
    }

    /// Marks the column as an optimistic concurrency token.
    pub fn is_concurrency_token(mut self) -> Self {
        self.override_entry().is_concurrency_token = Some(true);
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

    /// Sets a raw SQL `DEFAULT` expression, e.g. `.has_default("1")` or
    /// `.has_default("CURRENT_TIMESTAMP")`. String literals must include their
    /// quotes (`"'pending'"`).
    pub fn has_default(mut self, sql: &str) -> Self {
        self.override_entry().default_value = Some(sql.to_string());
        self
    }

    /// Adds a column-level CHECK constraint. The constraint name is derived as
    /// `CK_<table>_<column>` at model-build time (so table renames apply).
    pub fn has_check(mut self, expression: &str) -> Self {
        self.override_entry().check = Some(expression.to_string());
        self
    }
}

// ---------------------------------------------------------------------------
// IEntityTypeConfiguration<T>
// ---------------------------------------------------------------------------

pub trait IEntityTypeConfiguration<T: IEntityType> {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, T>);
}
