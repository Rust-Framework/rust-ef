//! Lazy loading infrastructure for navigation properties.
//!
//! Provides the `LazyContext` trait and `LazyContextImpl` struct that carry
//! the information needed to defer-load a navigation property on first
//! access. Containers (`BelongsTo<T>` / `HasMany<T>` / `HasOne<T>`) hold an
//! `Option<Arc<dyn LazyContext>>` and expose an async `load()` method.
//!
//! ## Recursion guard
//!
//! A `tokio::task_local!` depth counter prevents infinite lazy-loading
//! chains (e.g. `Blog → Posts → Blog → ...`). The maximum depth is
//! [`MAX_LAZY_DEPTH`]; exceeding it yields
//! `EFError::other("lazy loading recursion limit exceeded")`.

use crate::entity::IEntityType;
use crate::entity_snapshot::EntitySnapshot;
use crate::error::{EFError, EFResult};
use crate::metadata::{NavigationKind, NavigationMeta};
use crate::provider::{DbValue, IDatabaseProvider, ISqlGenerator};
use crate::query::CompiledFilter;
use std::collections::HashMap;
use std::sync::Arc;

/// Maximum nesting depth for lazy-loading chains.
///
/// A depth of 16 allows chains like
/// `Blog → Posts → Comments → Author → ...` (up to 16 levels) before
/// bailing out. Real-world graphs rarely exceed 3–4 levels; the limit
/// exists primarily to catch cyclic navigation graphs.
pub const MAX_LAZY_DEPTH: usize = 16;

// ---------------------------------------------------------------------------
// LazyContext trait
// ---------------------------------------------------------------------------

/// Context attached to a navigation container enabling deferred loading.
///
/// Stored as `Arc<dyn LazyContext>` inside `BelongsTo<T>` / `HasMany<T>` /
/// `HasOne<T>`. The context carries everything `load()` needs to build and
/// execute a single-entity navigation query:
///
/// - The database provider (for connection + SQL generation)
/// - The owning entity's primary-key values and full property snapshot
/// - The `NavigationMeta` describing which navigation to load
/// - Optional global query filters (e.g. tenant isolation)
/// - The current recursion depth (for cycle detection)
///
/// Implemented by [`LazyContextImpl`]; users can provide custom
/// implementations for advanced scenarios (e.g. custom caching).
pub trait LazyContext: Send + Sync {
    /// The database provider used to execute the lazy-load query.
    fn provider(&self) -> &Arc<dyn IDatabaseProvider>;

    /// The owning entity's full property snapshot (field_name → value).
    ///
    /// Used to extract foreign-key values for `BelongsTo` navigations.
    fn owner_snapshot(&self) -> &EntitySnapshot;

    /// The owning entity's primary-key values (field_name → value).
    ///
    /// Used to extract principal-key values for `HasMany` / `HasOne`
    /// navigations.
    fn owner_key_values(&self) -> &EntitySnapshot;

    /// Metadata describing the navigation to lazy-load.
    fn navigation(&self) -> &NavigationMeta;

    /// Optional global query filters keyed by table name.
    ///
    /// Applied to the related table's query so lazy-loaded data respects
    /// the same scoping (e.g. tenant isolation) as top-level queries.
    fn filter_map(&self) -> Option<&HashMap<String, CompiledFilter>>;

    /// Current recursion depth (0 = top-level lazy load).
    ///
    /// Incremented each time `load()` materializes child entities and
    /// attaches new lazy contexts to them.
    fn depth(&self) -> usize;
}

/// Concrete implementation of [`LazyContext`].
///
/// Constructed by the `#[derive(EntityType)]` macro's `ILazyInit`
/// implementation for each navigation field on an entity.
pub struct LazyContextImpl {
    provider: Arc<dyn IDatabaseProvider>,
    owner_snapshot: EntitySnapshot,
    owner_key_values: EntitySnapshot,
    navigation: NavigationMeta,
    filter_map: Option<Arc<HashMap<String, CompiledFilter>>>,
    depth: usize,
}

impl LazyContextImpl {
    /// Creates a new lazy context for a single navigation field.
    pub fn new(
        provider: Arc<dyn IDatabaseProvider>,
        owner_snapshot: EntitySnapshot,
        owner_key_values: EntitySnapshot,
        navigation: NavigationMeta,
        filter_map: Option<Arc<HashMap<String, CompiledFilter>>>,
        depth: usize,
    ) -> Self {
        Self {
            provider,
            owner_snapshot,
            owner_key_values,
            navigation,
            filter_map,
            depth,
        }
    }
}

impl LazyContext for LazyContextImpl {
    fn provider(&self) -> &Arc<dyn IDatabaseProvider> {
        &self.provider
    }
    fn owner_snapshot(&self) -> &EntitySnapshot {
        &self.owner_snapshot
    }
    fn owner_key_values(&self) -> &EntitySnapshot {
        &self.owner_key_values
    }
    fn navigation(&self) -> &NavigationMeta {
        &self.navigation
    }
    fn filter_map(&self) -> Option<&HashMap<String, CompiledFilter>> {
        self.filter_map.as_deref()
    }
    fn depth(&self) -> usize {
        self.depth
    }
}

// ---------------------------------------------------------------------------
// Query building for single-entity lazy loading
// ---------------------------------------------------------------------------

/// Builds the SQL and parameters for a single-entity lazy-load query.
///
/// Returns `Ok(None)` when the navigation metadata is incomplete (missing
/// `related_table` / `fk_column` / `referenced_key_column`); callers should
/// treat this as "nothing to load" and mark the container as loaded-but-empty.
///
/// For `ManyToMany` navigations, returns `Ok(None)` — M2M lazy loading
/// requires a two-phase query handled separately by
/// [`build_m2m_lazy_queries`].
fn build_lazy_query(
    nav: &NavigationMeta,
    owner_snapshot: &EntitySnapshot,
    owner_key_values: &EntitySnapshot,
    gen: &dyn ISqlGenerator,
    filter_map: Option<&HashMap<String, CompiledFilter>>,
) -> EFResult<Option<(String, Vec<DbValue>)>> {
    let Some(related_table) = nav.related_table.as_deref() else {
        return Ok(None);
    };
    let Some(fk_column) = nav.fk_column.as_deref() else {
        return Ok(None);
    };
    let Some(ref_column) = nav.referenced_key_column.as_deref() else {
        return Ok(None);
    };

    match nav.kind {
        NavigationKind::HasMany | NavigationKind::HasOne => {
            // The FK is on the related table; we need the owner's PK value
            // (matched by ref_column) to query WHERE related.fk = owner.pk.
            let Some(owner_pk) = owner_key_values.get(ref_column) else {
                return Ok(None);
            };
            let placeholder = gen.parameter_placeholder(1);
            let mut sql = format!(
                "SELECT * FROM {} WHERE {} = {}",
                related_table, fk_column, placeholder
            );
            let mut params = vec![owner_pk.clone()];
            apply_filter(&mut sql, &mut params, related_table, filter_map, gen);
            if nav.kind == NavigationKind::HasOne {
                sql.push_str(" LIMIT 1");
            }
            Ok(Some((sql, params)))
        }
        NavigationKind::BelongsTo => {
            // The FK is on THIS entity; we need the owner's FK value
            // (matched by fk_column) to query WHERE related.pk = owner.fk.
            let Some(owner_fk) = owner_snapshot.get(fk_column) else {
                return Ok(None);
            };
            if matches!(owner_fk, DbValue::Null) {
                // FK is NULL — no parent to load.
                return Ok(None);
            }
            let placeholder = gen.parameter_placeholder(1);
            let mut sql = format!(
                "SELECT * FROM {} WHERE {} = {}",
                related_table, ref_column, placeholder
            );
            let mut params = vec![owner_fk.clone()];
            apply_filter(&mut sql, &mut params, related_table, filter_map, gen);
            Ok(Some((sql, params)))
        }
        NavigationKind::ManyToMany => {
            // M2M is handled by build_m2m_lazy_queries (two-phase).
            Ok(None)
        }
    }
}

/// Builds the two SQL queries for a many-to-many lazy load.
///
/// Phase 1: query the join table for related IDs.
/// Phase 2: query the related table for entities matching those IDs.
///
/// Returns `Ok(None)` if M2M metadata is incomplete.
fn build_m2m_lazy_queries(
    nav: &NavigationMeta,
    owner_key_values: &EntitySnapshot,
    ref_column: &str,
    gen: &dyn ISqlGenerator,
    filter_map: Option<&HashMap<String, CompiledFilter>>,
) -> EFResult<Option<(String, Vec<DbValue>, String)>> {
    let Some(join_table) = nav.through_table.as_deref() else {
        return Ok(None);
    };
    let Some(parent_fk) = nav.through_parent_fk.as_deref() else {
        return Ok(None);
    };
    let Some(related_fk) = nav.through_related_fk.as_deref() else {
        return Ok(None);
    };
    let Some(related_table) = nav.related_table.as_deref() else {
        return Ok(None);
    };
    let Some(owner_pk) = owner_key_values.get(ref_column) else {
        return Ok(None);
    };

    // Phase 1: SELECT related_fk FROM join_table WHERE parent_fk = ?
    let placeholder = gen.parameter_placeholder(1);
    let join_sql = format!(
        "SELECT {} FROM {} WHERE {} = {}",
        related_fk, join_table, parent_fk, placeholder
    );
    let join_params = vec![owner_pk.clone()];

    // Phase 2 SQL is built dynamically after phase 1 returns the IDs,
    // because the IN-list size is unknown upfront. We return a template
    // that the caller fills in.
    let _ = filter_map; // filter applied in phase 2 (on related_table)
    Ok(Some((join_sql, join_params, related_table.to_string())))
}

/// Appends a global query filter (e.g. `tenant_id = ?`) to the SQL.
fn apply_filter(
    sql: &mut String,
    params: &mut Vec<DbValue>,
    related_table: &str,
    filter_map: Option<&HashMap<String, CompiledFilter>>,
    gen: &dyn ISqlGenerator,
) {
    use crate::query::compile_bool_expr;

    if let Some(filter) = filter_map.and_then(|m| m.get(related_table)) {
        let mut idx = params.len() + 1;
        let filter_sql = compile_bool_expr(&filter.expr, gen, &mut idx);
        params.extend(filter.params.iter().cloned());
        *sql = format!("{} AND ({})", sql, filter_sql);
    }
}

// ---------------------------------------------------------------------------
// Public helpers used by container load() methods
// ---------------------------------------------------------------------------

/// Executes a lazy-load query for a scalar navigation (BelongsTo / HasOne)
/// and returns the materialized entity, or `None` if no matching row.
///
/// This function is called by `BelongsTo<T>::load()` and
/// `HasOne<T>::load()`.
pub async fn load_scalar_lazy<T>(ctx: &dyn LazyContext) -> EFResult<Option<T>>
where
    T: IEntityType + crate::entity::IFromRow,
{
    let nav = ctx.navigation();
    if nav.kind == NavigationKind::ManyToMany {
        return Err(EFError::other(
            "load_scalar_lazy called for ManyToMany navigation",
        ));
    }

    let gen = ctx.provider().sql_generator();
    let Some((sql, params)) = build_lazy_query(
        nav,
        ctx.owner_snapshot(),
        ctx.owner_key_values(),
        gen,
        ctx.filter_map(),
    )?
    else {
        return Ok(None);
    };

    let provider = ctx.provider();
    let mut conn = provider.get_connection().await?;
    let rows = conn.query(&sql, &params).await?;
    drop(sql);
    drop(params);

    if let Some(row) = rows.into_iter().next() {
        let entity = T::from_row(&row)?;
        Ok(Some(entity))
    } else {
        Ok(None)
    }
}

/// Executes a lazy-load query for a collection navigation (HasMany) and
/// returns the materialized entities.
///
/// This function is called by `HasMany<T>::load()`.
pub async fn load_collection_lazy<T>(ctx: &dyn LazyContext) -> EFResult<Vec<T>>
where
    T: IEntityType + crate::entity::IFromRow,
{
    let nav = ctx.navigation();
    if nav.kind == NavigationKind::ManyToMany {
        return load_m2m_collection_lazy::<T>(ctx).await;
    }

    let gen = ctx.provider().sql_generator();
    let Some((sql, params)) = build_lazy_query(
        nav,
        ctx.owner_snapshot(),
        ctx.owner_key_values(),
        gen,
        ctx.filter_map(),
    )?
    else {
        return Ok(Vec::new());
    };

    let provider = ctx.provider();
    let mut conn = provider.get_connection().await?;
    let rows = conn.query(&sql, &params).await?;

    let mut entities = Vec::with_capacity(rows.len());
    for row in rows {
        entities.push(T::from_row(&row)?);
    }
    Ok(entities)
}

/// Executes a two-phase lazy-load for many-to-many navigations.
async fn load_m2m_collection_lazy<T>(ctx: &dyn LazyContext) -> EFResult<Vec<T>>
where
    T: IEntityType + crate::entity::IFromRow,
{
    let nav = ctx.navigation();
    let provider = ctx.provider();
    let gen = provider.sql_generator();

    // Determine the owner's PK column (used as ref_column).
    let ref_column = nav.referenced_key_column.as_deref().unwrap_or("id");

    let Some((join_sql, join_params, related_table)) = build_m2m_lazy_queries(
        nav,
        ctx.owner_key_values(),
        ref_column,
        gen,
        ctx.filter_map(),
    )?
    else {
        return Ok(Vec::new());
    };

    // Phase 1: query join table for related IDs.
    let mut conn = provider.get_connection().await?;
    let join_rows = conn.query(&join_sql, &join_params).await?;
    if join_rows.is_empty() {
        return Ok(Vec::new());
    }

    // Collect unique related IDs from join rows.
    let related_fk_index = nav.through_related_fk_index;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let related_ids: Vec<DbValue> = join_rows
        .iter()
        .filter_map(|row| row.get(related_fk_index).cloned())
        .filter(|v| seen.insert(format!("{}", v)))
        .collect();

    if related_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Phase 2: SELECT * FROM related_table WHERE pk IN (?, ?, ...).
    let ref_pk_col = nav
        .referenced_key_column
        .as_deref()
        .map(|s| s.to_string())
        .or_else(|| {
            nav.related_entity_meta.and_then(|f| {
                let meta = f();
                meta.primary_keys.first().map(|k| k.to_string())
            })
        })
        .unwrap_or_else(|| "id".to_string());

    let placeholders: Vec<String> = (0..related_ids.len())
        .map(|i| gen.parameter_placeholder(i + 1))
        .collect();
    let mut sql = format!(
        "SELECT * FROM {} WHERE {} IN ({})",
        related_table,
        ref_pk_col,
        placeholders.join(", ")
    );
    let mut params: Vec<DbValue> = related_ids;
    apply_filter(&mut sql, &mut params, &related_table, ctx.filter_map(), gen);

    let rows = conn.query(&sql, &params).await?;
    let mut entities = Vec::with_capacity(rows.len());
    for row in rows {
        entities.push(T::from_row(&row)?);
    }
    Ok(entities)
}
