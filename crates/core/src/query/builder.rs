//! `QueryBuilder<T>` — chainable query builder for entity type `T`.
//!
//! Corresponds to EFCore's `IQueryable<T>`. Accumulates filter conditions,
//! orderings, pagination, includes, projections, and CTE/window/set-op
//! state via a fluent interface. Terminal methods (`to_list`, `first`,
//! `count`, etc.) compile the state into SQL and execute it against the
//! attached provider.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::{
    IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, ILazyInit, INavigationSetter,
};
use crate::error::EFResult;
use crate::provider::{DbValue, DbValueConvertError, IDatabaseProvider};

use super::ast::{
    AggKind, BoolExpr, CompareOp, CompiledFilter, FilterCondition, HavingExpr, InSubquerySpec,
    IncludePath, JoinSpec, OrderBy, OrderDirection, SubquerySpec,
};
use super::compile::{
    build_where_clauses, collect_bool_expr_values, compile_bool_expr, convert_aggregate_cell,
    filters_to_and_expr, has_subqueries, resolve_subqueries,
};
use super::cte::{CteSpec, SetOpSpec, SetOperator};
use super::execute_update::ExecuteUpdateBuilder;
use super::select::SelectQueryBuilder;
use super::state::QueryState;
use super::window::{WindowFuncKind, WindowSpec};

/// A chainable query builder for entity type `T`.
///
/// Corresponds to EFCore's `IQueryable<T>`.
///
/// `Clone` is derived so that builders can be forked for compositional reuse
/// (e.g. applying additional filters on a base query without losing the
/// original). Note that `single`/`single_or_default` still use the `take(2)`
/// approach rather than `clone().count()` to avoid a double round-trip.
#[derive(Clone)]
pub struct QueryBuilder<T: IEntityType> {
    state: QueryState,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    filter_map: Option<Arc<HashMap<String, CompiledFilter>>>,
    lazy_loading_enabled: bool,
    _phantom: PhantomData<T>,
}

impl<T: IEntityType> QueryBuilder<T> {
    /// Creates a new QueryBuilder for a given table (without provider — SQL-only).
    pub fn new(table_name: impl Into<String>) -> Self {
        Self {
            state: QueryState::new(table_name),
            provider: None,
            filter_map: None,
            lazy_loading_enabled: false,
            _phantom: PhantomData,
        }
    }

    /// Creates a new QueryBuilder for a given table with a provider for execution.
    pub fn with_provider(
        table_name: impl Into<String>,
        provider: Arc<dyn IDatabaseProvider>,
    ) -> Self {
        Self {
            state: QueryState::new(table_name),
            provider: Some(provider),
            filter_map: None,
            lazy_loading_enabled: false,
            _phantom: PhantomData,
        }
    }

    /// Attaches a global filter map (table_name → BoolExpr) for NavigationLoader.
    pub(crate) fn with_filter_map(
        mut self,
        map: Option<Arc<HashMap<String, CompiledFilter>>>,
    ) -> Self {
        self.filter_map = map;
        self
    }

    /// Sets whether lazy loading is enabled for materialized entities.
    pub(crate) fn with_lazy_loading(mut self, enabled: bool) -> Self {
        self.lazy_loading_enabled = enabled;
        self
    }

    /// Returns a reference to the accumulated query state.
    pub fn state(&self) -> &QueryState {
        &self.state
    }

    /// Applies a compile-time LINQ expression tree from `linq!(?)`.
    pub fn filter(self, f: impl FnOnce(Self) -> Self) -> Self {
        f(self)
    }

    // -------------------------------------------------------------------
    // `linq!` expansion targets (`#[doc(hidden)]`)
    // -------------------------------------------------------------------

    #[doc(hidden)]
    pub fn filter_column(
        mut self,
        column: &str,
        operator: &str,
        value: impl Into<DbValue>,
    ) -> Self {
        let db_val = value.into();
        self.state.parameters.push(db_val);
        self.state
            .append_filter(FilterCondition::new(column, operator, 1));
        self
    }

    #[doc(hidden)]
    pub fn filter_not(mut self, column: &str, operator: &str, value: impl Into<DbValue>) -> Self {
        let db_val = value.into();
        self.state.parameters.push(db_val);
        self.state
            .append_bool_expr(BoolExpr::Not(Box::new(BoolExpr::Filter(
                FilterCondition::new(column, operator, 1),
            ))));
        self
    }

    #[doc(hidden)]
    pub fn filter_in(mut self, column: &str, values: Vec<DbValue>) -> Self {
        let count = values.len();
        for v in values {
            self.state.parameters.push(v);
        }
        self.state
            .append_filter(FilterCondition::new(column, "IN", count));
        self
    }

    #[doc(hidden)]
    pub fn filter_not_in(mut self, column: &str, values: Vec<DbValue>) -> Self {
        let count = values.len();
        for v in values {
            self.state.parameters.push(v);
        }
        self.state
            .append_bool_expr(BoolExpr::Not(Box::new(BoolExpr::Filter(
                FilterCondition::new(column, "IN", count),
            ))));
        self
    }

    #[doc(hidden)]
    pub fn filter_is_null(mut self, column: &str) -> Self {
        self.state
            .append_filter(FilterCondition::new(column, "IS NULL", 0));
        self
    }

    #[doc(hidden)]
    pub fn filter_is_not_null(mut self, column: &str) -> Self {
        self.state
            .append_filter(FilterCondition::new(column, "IS NOT NULL", 0));
        self
    }

    #[doc(hidden)]
    pub fn filter_between(
        mut self,
        column: &str,
        low: impl Into<DbValue>,
        high: impl Into<DbValue>,
    ) -> Self {
        let lo: DbValue = low.into();
        let hi: DbValue = high.into();
        self.state.parameters.push(lo);
        self.state.parameters.push(hi);
        self.state
            .append_filter(FilterCondition::new(column, "BETWEEN", 2));
        self
    }

    #[doc(hidden)]
    pub fn filter_like(self, column: &str, pattern: impl Into<DbValue>) -> Self {
        self.filter_column(column, "LIKE", pattern)
    }

    #[doc(hidden)]
    pub fn filter_not_like(self, column: &str, pattern: impl Into<DbValue>) -> Self {
        self.filter_not(column, "LIKE", pattern)
    }

    #[doc(hidden)]
    pub fn order_by_column(mut self, column: &str) -> Self {
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Ascending));
        self
    }

    #[doc(hidden)]
    pub fn order_by_desc_column(mut self, column: &str) -> Self {
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Descending));
        self
    }

    /// Applies a global query filter `BoolExpr` (produced by `linq!(filter |b: T| ...)`).
    /// Inline values carried by the expression are collected and appended to
    /// the query parameters in the correct position.
    pub(crate) fn apply_query_filter(mut self, filter: BoolExpr) -> Self {
        let values = collect_bool_expr_values(&filter);
        self.state.parameters.extend(values);
        self.state.append_bool_expr(filter);
        self
    }

    /// Marks this query as `SELECT DISTINCT`.
    pub fn distinct(mut self) -> Self {
        self.state.distinct = true;
        self
    }

    #[doc(hidden)]
    pub fn or_where(mut self, f: impl FnOnce(QueryBuilder<T>) -> QueryBuilder<T>) -> Self {
        let sub = f(QueryBuilder {
            state: QueryState::new(&self.state.from),
            provider: self.provider.clone(),
            filter_map: None,
            lazy_loading_enabled: false,
            _phantom: PhantomData,
        });
        let right = sub.state.where_expr.or_else(|| {
            if sub.state.filters.is_empty() {
                None
            } else {
                Some(filters_to_and_expr(&sub.state.filters))
            }
        });
        if let Some(right_expr) = right {
            self.state.where_expr = Some(match self.state.where_expr.take() {
                None => right_expr,
                Some(left) => BoolExpr::Or(Box::new(left), Box::new(right_expr)),
            });
            self.state.parameters.extend(sub.state.parameters);
            self.state.filters.extend(sub.state.filters);
        }
        self
    }

    /// G5: Adds an `EXISTS` (or `NOT EXISTS`) correlated subquery condition.
    ///
    /// `#[doc(hidden)]` — called by `linq!` expansion of
    /// `b.posts.any(|p| p.published)` / `b.posts.none(...)` / `b.posts.all(...)`.
    ///
    /// The `nav_field` and `related_type` arguments are the `&'static str`
    /// constants emitted by `#[derive(EntityType)]` (`FIELD_<NAME>` and
    /// `NAV_RELATED_<NAME>`). The table/column fields of the `SubquerySpec`
    /// are resolved later at SQL generation time via `resolve_subqueries`.
    pub fn where_exists_internal(
        mut self,
        nav_field: &'static str,
        related_type: &'static str,
        predicate: Option<BoolExpr>,
        negated: bool,
    ) -> Self {
        let mut spec = SubquerySpec::new(nav_field, related_type);
        if let Some(pred) = predicate {
            // Collect self-contained values from the predicate (e.g. the
            // `DbValue::Bool(true)` from `p.published`) and append them to
            // the query parameters. The `?` placeholders generated by
            // `compile_bool_expr` will reference them in order.
            let values = collect_bool_expr_values(&pred);
            self.state.parameters.extend(values);
            spec.predicate = Some(Box::new(pred));
        }
        let expr = if negated {
            BoolExpr::NotExists(Box::new(spec))
        } else {
            BoolExpr::Exists(Box::new(spec))
        };
        self.state.append_bool_expr(expr);
        self
    }

    /// v1.1: Adds an `IN (SELECT ...)` (or `NOT IN (SELECT ...)`) subquery
    /// condition.
    ///
    /// `#[doc(hidden)]` — called by `linq!` expansion of
    /// `b.field.in_subquery(|p: Post| p.blog_id)`.
    ///
    /// Unlike `where_exists_internal`, the `InSubquerySpec` is fully
    /// specified at construction time (no navigation resolution needed).
    /// The `source_table` and `projection_column` are `&'static str`
    /// constants emitted by `#[derive(EntityType)]` (`TABLE` and
    /// `COLUMN_<NAME>`).
    pub fn where_in_subquery_internal(
        mut self,
        outer_column: &'static str,
        source_table: &'static str,
        projection_column: &'static str,
        predicate: Option<BoolExpr>,
        negated: bool,
    ) -> Self {
        let mut spec = InSubquerySpec::new(outer_column, source_table, projection_column);
        if let Some(pred) = predicate {
            let values = collect_bool_expr_values(&pred);
            self.state.parameters.extend(values);
            spec.predicate = Some(Box::new(pred));
        }
        let expr = if negated {
            BoolExpr::NotInSubquery(Box::new(spec))
        } else {
            BoolExpr::InSubquery(Box::new(spec))
        };
        self.state.append_bool_expr(expr);
        self
    }

    // -------------------------------------------------------------------
    // Chainable methods (each returns Self with accumulated state)
    // -------------------------------------------------------------------

    /// Finds an entity by its single primary key. Uses the entity's PK
    /// metadata — no longer hardcodes `"id"`.
    pub async fn find(self, id: impl Into<DbValue>) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let meta = T::entity_meta();
        let pk_col = meta
            .primary_keys
            .first()
            .map(|s| s.as_ref())
            .or_else(|| {
                meta.properties
                    .iter()
                    .find(|p| p.is_primary_key)
                    .map(|p| p.column_name.as_ref())
            })
            .ok_or_else(|| {
                crate::error::EFError::Query(format!(
                    "entity {} has no primary key defined",
                    std::any::type_name::<T>()
                ))
            })?;
        let col_const = pk_col.to_string();
        self.filter_column(&col_const, "=", id)
            .first_or_default()
            .await
    }

    /// Finds an entity by composite primary key. Keys are column-name
    /// constants paired with values, e.g. `&[(BlogTag::COLUMN_BLOG_ID, DbValue::I32(1))]`.
    pub async fn find_by_key(mut self, keys: &[(&str, DbValue)]) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        for (col, val) in keys {
            self = self.filter_column(col, "=", val.clone());
        }
        self.first_or_default().await
    }

    /// Checks if an entity with the given single primary key exists.
    ///
    /// Uses `SELECT 1 ... LIMIT 1` — cheaper than `find(id).await?.is_some()`
    /// which materializes the full row. Reads the PK column from entity
    /// metadata, mirroring [`find`](Self::find).
    pub async fn exists_by_id(self, id: impl Into<DbValue>) -> EFResult<bool> {
        let meta = T::entity_meta();
        let pk_col = meta
            .primary_keys
            .first()
            .map(|s| s.as_ref())
            .or_else(|| {
                meta.properties
                    .iter()
                    .find(|p| p.is_primary_key)
                    .map(|p| p.column_name.as_ref())
            })
            .ok_or_else(|| {
                crate::error::EFError::Query(format!(
                    "entity {} has no primary key defined",
                    std::any::type_name::<T>()
                ))
            })?;
        let col_const = pk_col.to_string();
        self.filter_column(&col_const, "=", id).any().await
    }

    /// Checks if an entity with the given composite key exists.
    ///
    /// Uses `SELECT 1 ... LIMIT 1` — cheaper than `find_by_key(keys).is_some()`.
    pub async fn exists_by_key(mut self, keys: &[(&str, DbValue)]) -> EFResult<bool> {
        for (col, val) in keys {
            self = self.filter_column(col, "=", val.clone());
        }
        self.any().await
    }

    /// Skips the specified number of rows.
    pub fn skip(mut self, count: usize) -> Self {
        self.state.offset = Some(count);
        self
    }

    /// Takes the specified number of rows.
    pub fn take(mut self, count: usize) -> Self {
        self.state.limit = Some(count);
        self
    }

    /// Eagerly loads a named navigation (resolves FK/table from entity metadata).
    ///
    /// `#[doc(hidden)]` — called by `linq!(include b.posts)` expansion. Users
    /// should use the `linq!` macro instead of calling this directly.
    #[doc(hidden)]
    pub fn include_internal(mut self, navigation: &'static str) -> Self {
        let meta = T::entity_meta();
        let nav_meta = meta.find_navigation(navigation);
        let (related_table, fk_col, ref_col) = nav_meta
            .map(|n| {
                (
                    n.related_table.as_ref().map(|s| s.to_string()),
                    n.fk_column.as_ref().map(|s| s.to_string()),
                    n.referenced_key_column.as_ref().map(|s| s.to_string()),
                )
            })
            .unwrap_or((None, None, None));

        self.state.includes.push(IncludePath {
            navigation: navigation.to_string(),
            nested: Vec::new(),
            related_table,
            foreign_key_column: fk_col,
            referenced_key_column: ref_col,
        });
        self
    }

    /// Eagerly loads a nested navigation on the last `include_internal` path.
    ///
    /// `#[doc(hidden)]` — called by `linq!(include b.posts then b.comments)`
    /// expansion. The nested navigation field name is a string literal because
    /// the entity type transition is runtime knowledge (resolved via metadata).
    #[doc(hidden)]
    pub fn then_include_internal(mut self, navigation: &'static str) -> Self {
        if let Some(last) = self.state.includes.last_mut() {
            let parent_meta = T::entity_meta();
            if let Some(parent_nav) = parent_meta.find_navigation(&last.navigation) {
                if let Some(meta_fn) = parent_nav.related_entity_meta {
                    let related_meta = meta_fn();
                    if let Some(nav_meta) = related_meta.find_navigation(navigation) {
                        last.nested.push(IncludePath {
                            navigation: navigation.to_string(),
                            nested: Vec::new(),
                            related_table: nav_meta.related_table.as_ref().map(|s| s.to_string()),
                            foreign_key_column: nav_meta.fk_column.as_ref().map(|s| s.to_string()),
                            referenced_key_column: nav_meta
                                .referenced_key_column
                                .as_ref()
                                .map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
        self
    }

    /// Adds an INNER JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(inner_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn inner_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "INNER".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a LEFT JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(left_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn left_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "LEFT".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a RIGHT JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(right_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn right_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "RIGHT".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a FULL OUTER JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(full_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    ///
    /// Note: MySQL does not support FULL OUTER JOIN; the SQL will fail at
    /// execution time on MySQL providers.
    #[doc(hidden)]
    pub fn full_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "FULL".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a CROSS JOIN (no ON condition).
    ///
    /// `#[doc(hidden)]` — called by `linq!(cross_join b: T2)` expansion.
    #[doc(hidden)]
    pub fn cross_join_internal(mut self, table: &'static str) -> Self {
        self.state.joins.push(JoinSpec {
            join_type: "CROSS".to_string(),
            table: table.to_string(),
            on_clause: String::new(),
        });
        self
    }

    /// Adds a GROUP BY clause.
    ///
    /// `#[doc(hidden)]` — called by `linq!(group_by (b.cat, b.author))` expansion.
    #[doc(hidden)]
    pub fn group_by_internal(mut self, columns: &'static [&'static str]) -> Self {
        self.state.group_bys = columns.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Adds a HAVING condition.
    ///
    /// `#[doc(hidden)]` — called by `linq!(having count(b.id) > 1)` expansion.
    /// Constructs `agg(column) op ?` with the value pushed to parameters.
    #[doc(hidden)]
    pub fn having_internal(
        mut self,
        agg: &str,
        column: &str,
        op: &str,
        value: impl Into<DbValue>,
    ) -> Self {
        let agg_kind = AggKind::from_name(agg)
            .unwrap_or_else(|| panic!("invalid aggregate name in having_internal: {agg}"));
        let cmp_op = CompareOp::from_symbol(op)
            .unwrap_or_else(|| panic!("invalid operator in having_internal: {op}"));
        let db_val = value.into();
        self.state.parameters.push(db_val.clone());
        self.state.havings.push(HavingExpr::Compare {
            agg: agg_kind,
            col: column.to_string(),
            op: cmp_op,
            value: db_val,
        });
        self
    }

    /// Adds a HAVING condition from a `HavingExpr` AST.
    ///
    /// `#[doc(hidden)]` — called by `linq!(having <expr>)` expansion when the
    /// having clause contains boolean combinations (`AND`/`OR`/`NOT`) or
    /// aggregate-versus-aggregate comparisons. The expression is stored as an
    /// AST node and compiled to SQL at `to_sql_with` time using the provider's
    /// placeholder syntax; bound parameters are collected via
    /// [`HavingExpr::collect_params`] and pushed to `state.parameters`.
    #[doc(hidden)]
    pub fn having_expr_internal(mut self, expr: HavingExpr) -> Self {
        self.state.parameters.extend(expr.collect_params());
        self.state.havings.push(expr);
        self
    }

    // -------------------------------------------------------------------
    // Window functions & CTE (v1.1)
    // -------------------------------------------------------------------

    /// Adds a window function projection to the SELECT list.
    ///
    /// `#[doc(hidden)]` — called by `linq!(window ...)` expansion.
    ///
    /// - `func`: window function name (e.g. `"row_number"`, `"sum"`, `"lag"`).
    /// - `column`: the column argument (`None` for ranking functions).
    /// - `partition_by`: PARTITION BY columns.
    /// - `order_by`: ORDER BY columns as `(column, descending)` pairs.
    /// - `alias`: the output column alias.
    #[doc(hidden)]
    pub fn window_internal(
        mut self,
        func: &str,
        column: Option<&str>,
        partition_by: &'static [&'static str],
        order_by: &'static [(&'static str, bool)],
        alias: &str,
    ) -> Self {
        let kind = WindowFuncKind::from_name(func)
            .unwrap_or_else(|| panic!("invalid window function name: {func}"));
        if kind.takes_column() && column.is_none() {
            panic!("window function `{func}` requires a column argument");
        }
        let spec = WindowSpec {
            func: kind,
            column: column.map(|s| s.to_string()),
            partition_by: partition_by.iter().map(|s| s.to_string()).collect(),
            order_by: order_by
                .iter()
                .map(|(c, d)| {
                    (
                        c.to_string(),
                        if *d {
                            OrderDirection::Descending
                        } else {
                            OrderDirection::Ascending
                        },
                    )
                })
                .collect(),
            alias: alias.to_string(),
        };
        self.state.windows.push(spec);
        self
    }

    /// Adds a CTE (Common Table Expression) definition to the query (raw mode).
    ///
    /// `#[doc(hidden)]` — called by runtime API users.
    ///
    /// The CTE body is a pre-compiled SQL string with `?` placeholders; its
    /// parameter values are prepended to the query's parameter vector at
    /// execution time so that placeholder ordering remains contiguous.
    ///
    /// **Note**: Raw mode emits `?` placeholders verbatim and does not convert
    /// them to provider-specific syntax (`$N` on PostgreSQL). For
    /// provider-correct placeholders, use `with_cte_typed` (via
    /// `linq!(with ...)`).
    #[doc(hidden)]
    pub fn with_cte_internal(
        mut self,
        name: &str,
        sql: &str,
        params: Vec<DbValue>,
        columns: &'static [&'static str],
    ) -> Self {
        let cte = CteSpec {
            name: name.to_string(),
            sql: sql.to_string(),
            table: String::new(),
            where_expr: None,
            params,
            columns: columns.iter().map(|s| s.to_string()).collect(),
            is_recursive: false,
            recursive_link: None,
        };
        self.state.ctes.push(cte);
        self
    }

    /// Adds a typed CTE definition (typed mode), used by `linq!(with ...)`.
    ///
    /// `#[doc(hidden)]` — called by `linq!(with name as |e: T| ...)` expansion.
    ///
    /// The CTE body `SELECT * FROM <table> WHERE <where_expr>` is compiled at
    /// `to_sql_with` time using the provider's placeholder syntax, ensuring
    /// correct `$N` numbering on PostgreSQL and `?` on SQLite/MySQL.
    ///
    /// Parameter values are extracted from `where_expr` via
    /// `collect_bool_expr_values` and stored in `params` so that `all_params()`
    /// returns them in the correct order (CTE params first).
    #[doc(hidden)]
    pub fn with_cte_typed(mut self, name: &str, table: &str, where_expr: BoolExpr) -> Self {
        let params = collect_bool_expr_values(&where_expr);
        let cte = CteSpec {
            name: name.to_string(),
            sql: String::new(),
            table: table.to_string(),
            where_expr: Some(where_expr),
            params,
            columns: Vec::new(),
            is_recursive: false,
            recursive_link: None,
        };
        self.state.ctes.push(cte);
        self
    }

    /// Adds a typed recursive CTE definition, used by
    /// `linq!(with recursive name as |e: T| ...; link e.fk to e.pk)`.
    ///
    /// `#[doc(hidden)]` — called by `linq!(with recursive ...)` expansion.
    ///
    /// Generates `WITH RECURSIVE name AS (anchor UNION ALL SELECT t.* FROM
    /// <table> t JOIN name ON t.<link_fk> = name.<link_pk>)` where `anchor` is
    /// `SELECT * FROM <table> WHERE <where_expr>` (or `SELECT * FROM <table>`
    /// if `where_expr` is `BoolExpr::raw("")`).
    #[doc(hidden)]
    pub fn with_recursive_cte_typed(
        mut self,
        name: &str,
        table: &str,
        link_fk: &str,
        link_pk: &str,
        where_expr: BoolExpr,
    ) -> Self {
        let params = collect_bool_expr_values(&where_expr);
        let cte = CteSpec {
            name: name.to_string(),
            sql: String::new(),
            table: table.to_string(),
            where_expr: Some(where_expr),
            params,
            columns: Vec::new(),
            is_recursive: true,
            recursive_link: Some((link_fk.to_string(), link_pk.to_string())),
        };
        self.state.ctes.push(cte);
        self
    }

    /// Changes the FROM clause to reference a CTE name (or any table/subquery).
    ///
    /// Used in combination with `with_cte_internal` to query from a CTE:
    /// ```ignore
    /// builder.with_cte_internal("cte", "SELECT ...", params, &[])
    ///        .from_cte("cte")
    /// ```
    #[doc(hidden)]
    pub fn from_cte(mut self, name: &str) -> Self {
        self.state.from = name.to_string();
        self
    }

    // -------------------------------------------------------------------
    // Set operations (UNION / INTERSECT / EXCEPT)
    // -------------------------------------------------------------------

    /// Appends a UNION operand.
    ///
    /// `operand` is a `(sql, params)` tuple from `QueryBuilder::compile_sql()`.
    /// Per D5, the operand should not contain ORDER BY / LIMIT.
    #[doc(hidden)]
    pub fn union_internal(mut self, operand: (String, Vec<DbValue>)) -> Self {
        self.state.set_operations.push(SetOpSpec {
            operator: SetOperator::Union,
            operand_sql: operand.0,
            operand_params: operand.1,
        });
        self
    }

    /// Appends a UNION ALL operand.
    #[doc(hidden)]
    pub fn union_all_internal(mut self, operand: (String, Vec<DbValue>)) -> Self {
        self.state.set_operations.push(SetOpSpec {
            operator: SetOperator::UnionAll,
            operand_sql: operand.0,
            operand_params: operand.1,
        });
        self
    }

    /// Appends an INTERSECT operand.
    #[doc(hidden)]
    pub fn intersect_internal(mut self, operand: (String, Vec<DbValue>)) -> Self {
        self.state.set_operations.push(SetOpSpec {
            operator: SetOperator::Intersect,
            operand_sql: operand.0,
            operand_params: operand.1,
        });
        self
    }

    /// Appends an EXCEPT operand.
    #[doc(hidden)]
    pub fn except_internal(mut self, operand: (String, Vec<DbValue>)) -> Self {
        self.state.set_operations.push(SetOpSpec {
            operator: SetOperator::Except,
            operand_sql: operand.0,
            operand_params: operand.1,
        });
        self
    }

    // -------------------------------------------------------------------
    // Aggregate terminal methods
    // -------------------------------------------------------------------

    /// Executes a SUM aggregation query.
    ///
    /// `#[doc(hidden)]` — called by `linq!(sum b.views)` expansion.
    #[doc(hidden)]
    pub async fn sum_internal(self, column: &'static str) -> EFResult<f64> {
        let mut state = self.state.clone();
        state.aggregate = Some("SUM".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            f64::try_from(first.clone()).map_err(|_| {
                crate::error::EFError::TypeConversion("SUM result is not f64".to_string())
            })
        } else {
            Ok(0.0)
        }
    }

    /// Executes an AVG aggregation query.
    ///
    /// `#[doc(hidden)]` — called by `linq!(avg b.rating)` expansion.
    #[doc(hidden)]
    pub async fn avg_internal(self, column: &'static str) -> EFResult<f64> {
        let mut state = self.state.clone();
        state.aggregate = Some("AVG".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            f64::try_from(first.clone()).map_err(|_| {
                crate::error::EFError::TypeConversion("AVG result is not f64".to_string())
            })
        } else {
            Ok(0.0)
        }
    }

    /// Executes a MIN aggregation query, returning the typed result.
    ///
    /// `#[doc(hidden)]` — called by `linq!(min b.rating)` expansion. The target
    /// type `V` is inferred from the call site (e.g. `let v: i64 = ...`).
    #[doc(hidden)]
    pub async fn min_internal<V>(self, column: &'static str) -> EFResult<Option<V>>
    where
        V: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let mut state = self.state.clone();
        state.aggregate = Some("MIN".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        convert_aggregate_cell::<V>(rows)
    }

    /// Executes a MAX aggregation query, returning the typed result.
    ///
    /// `#[doc(hidden)]` — called by `linq!(max b.rating)` expansion. The target
    /// type `V` is inferred from the call site (e.g. `let v: i64 = ...`).
    #[doc(hidden)]
    pub async fn max_internal<V>(self, column: &'static str) -> EFResult<Option<V>>
    where
        V: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let mut state = self.state.clone();
        state.aggregate = Some("MAX".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        convert_aggregate_cell::<V>(rows)
    }

    // -------------------------------------------------------------------
    // Terminal methods
    // -------------------------------------------------------------------

    /// Projects to named columns and returns raw row values.
    ///
    /// `#[doc(hidden)]` — called by `linq!(select (b.id, b.title))` expansion.
    #[doc(hidden)]
    pub fn select_internal(self, columns: &'static [&'static str]) -> SelectQueryBuilder<T> {
        let mut state = self.state.clone();
        state.projected_columns = Some(columns.iter().map(|s| s.to_string()).collect());
        SelectQueryBuilder {
            state,
            provider: self.provider,
            _phantom: PhantomData,
        }
    }

    // -------------------------------------------------------------------
    // Terminal methods
    // -------------------------------------------------------------------

    /// Builds the SQL string for this query.
    pub fn to_sql(&self) -> String {
        // G5: Resolve subquery specs using entity metadata.
        let mut state = self.state.clone();
        if let Some(ref mut expr) = state.where_expr {
            if has_subqueries(expr) {
                let meta = T::entity_meta();
                resolve_subqueries(expr, &meta);
            }
        }
        if let Some(provider) = &self.provider {
            let gen = provider.sql_generator();
            state.to_sql_with(gen)
        } else {
            state.to_sql()
        }
    }

    pub fn compile_sql(&self) -> (String, Vec<DbValue>) {
        (self.to_sql(), self.state.all_params())
    }

    fn compile_state_sql(state: &QueryState, provider: &Arc<dyn IDatabaseProvider>) -> String {
        let gen = provider.sql_generator();
        // G5: Resolve subquery specs before SQL compilation.
        let mut resolved = state.clone();
        if let Some(ref mut expr) = resolved.where_expr {
            if has_subqueries(expr) {
                let meta = T::entity_meta();
                resolve_subqueries(expr, &meta);
            }
        }
        resolved.to_sql_with(gen)
    }

    /// Executes the query and returns all matching entities.
    pub async fn to_list(self) -> EFResult<Vec<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let includes = self.state.includes.clone();
        let lazy_loading = self.lazy_loading_enabled;
        let (sql, params) = self.compile_sql();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder. Use DbSet::query() or attach a provider."
                    .to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        let mut entities = crate::entity::materialize_entities::<T>(&rows)?;
        if !includes.is_empty() {
            crate::navigation_loader::load_includes(
                &mut entities,
                &includes,
                &**provider,
                self.filter_map.as_deref(),
            )
            .await?;
        }
        // When lazy loading is enabled and no explicit includes were
        // requested, attach a LazyContext to each navigation container on
        // every materialized entity. The user can then call
        // `nav.load().await` to trigger on-demand loading.
        if lazy_loading && includes.is_empty() {
            let provider_arc = Arc::clone(provider);
            let filter_map = self.filter_map.clone();
            for entity in &mut entities {
                entity.attach_lazy_contexts(Arc::clone(&provider_arc), filter_map.clone(), 0);
            }
        }
        Ok(entities)
    }

    /// Executes the query and eagerly loads included navigations.
    pub async fn to_list_with_includes(self) -> EFResult<Vec<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        self.to_list().await
    }

    /// Executes the query and returns the first matching entity.
    pub async fn first(self) -> EFResult<T>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(1).to_list().await?;
        results
            .pop()
            .ok_or_else(|| crate::error::EFError::NotFound("Entity not found".to_string()))
    }

    /// Executes the query and returns the first matching entity or None.
    pub async fn first_or_default(self) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(1).to_list().await?;
        Ok(results.pop())
    }

    /// Executes a COUNT query.
    pub async fn count(self) -> EFResult<i64> {
        let mut state = self.state.clone();
        state.is_count = true;
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first_row) = rows.first() {
            if let Some(first_val) = first_row.first() {
                if matches!(first_val, crate::provider::DbValue::Null) {
                    return Ok(0);
                }
                return i64::try_from(first_val.clone()).map_err(|e| {
                    crate::error::EFError::TypeConversion(format!(
                        "COUNT result is not i64: {}",
                        e
                    ))
                });
            }
        }
        Ok(0)
    }

    /// Checks if any entities match the query.
    pub async fn any(self) -> EFResult<bool> {
        let mut state = self.state.clone();
        state.is_exists = true;
        state.limit = Some(1);
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        Ok(!rows.is_empty())
    }

    // -------------------------------------------------------------------
    // Additional LINQ terminal methods
    // -------------------------------------------------------------------

    /// Executes the query and returns the last matching entity (reverses
    /// ordering, then takes 1). Errors if no rows match.
    pub async fn last(self) -> EFResult<T>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.last_or_default().await?;
        results
            .take()
            .ok_or_else(|| crate::error::EFError::NotFound("Entity not found".to_string()))
    }

    /// Executes the query and returns the last matching entity or `None`.
    ///
    /// When the caller has set explicit `order_by` clauses, their directions
    /// are reversed and `take(1)` returns the last row under that ordering.
    /// When no ordering is set, a default `ORDER BY <pk> DESC` is injected so
    /// that "last" has deterministic semantics (matches the original design
    /// in the v0.4 plan §4 阶段 4). Errors if the entity has no primary key
    /// and no explicit ordering was provided.
    pub async fn last_or_default(mut self) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        if self.state.orderings.is_empty() {
            let meta = T::entity_meta();
            let pk_col = meta
                .primary_keys
                .first()
                .map(|s| s.as_ref())
                .or_else(|| {
                    meta.properties
                        .iter()
                        .find(|p| p.is_primary_key)
                        .map(|p| p.column_name.as_ref())
                })
                .ok_or_else(|| {
                    crate::error::EFError::Query(format!(
                        "last_or_default requires a primary key on {} when no explicit ordering is set",
                        std::any::type_name::<T>()
                    ))
                })?;
            self.state
                .orderings
                .push(OrderBy::new(pk_col.to_string(), OrderDirection::Descending));
        } else {
            // Reverse existing orderings to get the "last" row.
            for o in &mut self.state.orderings {
                o.direction = match o.direction {
                    OrderDirection::Ascending => OrderDirection::Descending,
                    OrderDirection::Descending => OrderDirection::Ascending,
                };
            }
        }
        let mut results = self.take(1).to_list().await?;
        Ok(results.pop())
    }

    /// Executes the query and returns the only matching entity. Errors if
    /// there are 0 or 2+ results.
    pub async fn single(self) -> EFResult<T>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(2).to_list().await?;
        if results.len() > 1 {
            return Err(crate::error::EFError::Query(
                "Sequence contains more than one element".to_string(),
            ));
        }
        results.pop().ok_or_else(|| {
            crate::error::EFError::NotFound("Sequence contains no elements".to_string())
        })
    }

    /// Executes the query and returns the only matching entity, or `None` if
    /// empty. Errors if there are 2+ results.
    pub async fn single_or_default(self) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(2).to_list().await?;
        if results.len() > 1 {
            return Err(crate::error::EFError::Query(
                "Sequence contains more than one element".to_string(),
            ));
        }
        Ok(results.pop())
    }

    /// Executes a COUNT query and returns the result as `i64`. Alias for
    /// `count()` — in .NET LINQ, `LongCount` returns `long` while `Count`
    /// returns `int`; in Rust both are `i64`.
    pub async fn long_count(self) -> EFResult<i64> {
        self.count().await
    }

    /// Determines whether all elements in the sequence satisfy a predicate.
    /// The predicate is applied in Rust after loading the entities.
    pub async fn all<F>(self, predicate: F) -> EFResult<bool>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
        F: Fn(&T) -> bool,
    {
        let items = self.to_list().await?;
        Ok(items.iter().all(predicate))
    }

    /// Determines whether the sequence contains an entity with the given
    /// primary key value.
    pub async fn contains(self, id: impl Into<DbValue>) -> EFResult<bool>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        self.find(id).await.map(|opt| opt.is_some())
    }

    /// Projects each entity into a key-value pair and collects into a
    /// `HashMap<K, T>`. The key selector closure extracts the key from each
    /// entity.
    pub async fn to_dictionary<K, F>(
        self,
        key_selector: F,
    ) -> EFResult<std::collections::HashMap<K, T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
        K: std::hash::Hash + Eq,
        F: Fn(&T) -> K,
    {
        let items = self.to_list().await?;
        let mut map = std::collections::HashMap::with_capacity(items.len());
        for item in items {
            let key = key_selector(&item);
            map.insert(key, item);
        }
        Ok(map)
    }

    // -------------------------------------------------------------------
    // Bulk operations (ExecuteUpdate / ExecuteDelete)
    // -------------------------------------------------------------------

    /// Prepares a bulk update operation.
    pub fn execute_update(self) -> ExecuteUpdateBuilder<T> {
        ExecuteUpdateBuilder {
            state: self.state.clone(),
            updates: Vec::new(),
            provider: self.provider.clone(),
            _phantom: PhantomData,
        }
    }

    /// Executes a bulk delete operation.
    pub async fn execute_delete(self) -> EFResult<u64> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let gen = provider.sql_generator();
        // G5: Resolve subqueries before compiling WHERE clause.
        let mut resolved_expr = self.state.where_expr.clone();
        if let Some(ref mut expr) = resolved_expr {
            if has_subqueries(expr) {
                let meta = T::entity_meta();
                resolve_subqueries(expr, &meta);
            }
        }
        let where_clause = if let Some(ref expr) = resolved_expr {
            let mut param_idx = 1usize;
            compile_bool_expr(expr, gen, &mut param_idx)
        } else {
            build_where_clauses(&self.state.filters, gen)
        };
        let sql = if where_clause.is_empty() {
            format!("DELETE FROM {}", self.state.from)
        } else {
            format!("DELETE FROM {} WHERE {}", self.state.from, where_clause)
        };
        let params = self.state.all_params();
        let mut conn = provider.get_connection().await?;
        conn.execute(&sql, &params).await
    }
}
