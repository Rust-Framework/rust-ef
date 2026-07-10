//! GROUP BY, HAVING, window functions, CTE, set operations, aggregates, and projection.

use std::marker::PhantomData;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::{DbValue, DbValueConvertError};

use super::super::ast::{BoolExpr, OrderDirection};
use super::super::compile::{collect_bool_expr_values, convert_aggregate_cell};
use super::super::cte::{CteSpec, SetOpSpec, SetOperator};
use super::super::having_expr::{AggKind, CompareOp, HavingExpr};
use super::super::select::SelectQueryBuilder;
use super::super::window::{WindowFuncKind, WindowSpec};
use super::core::QueryBuilder;

impl<T: IEntityType> QueryBuilder<T> {
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
            crate::error::EFError::configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            f64::try_from(first.clone()).map_err(|_| {
                crate::error::EFError::type_conversion("SUM result is not f64".to_string())
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
            crate::error::EFError::configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            f64::try_from(first.clone()).map_err(|_| {
                crate::error::EFError::type_conversion("AVG result is not f64".to_string())
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
            crate::error::EFError::configuration(
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
            crate::error::EFError::configuration(
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
    // Projection
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
}
