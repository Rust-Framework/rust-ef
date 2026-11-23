//! `SelectQueryBuilder<T>` — projection query builder.
//!
//! Produced by `QueryBuilder::select_internal` when the user projects to
//! named columns. Executes the projection query and returns either raw
//! `Vec<Vec<DbValue>>` rows or strongly-typed tuples via `to_list_typed_n`
//! methods (converting each column via `TryFrom<DbValue>`).

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::{DbValue, DbValueConvertError, IDatabaseProvider};
use crate::row::IRow;

use super::ast::{OrderBy, OrderDirection};
use super::compile::resolve_state_subqueries;
use super::expr::SelectItem;
use super::having_expr::HavingExpr;
use super::state::QueryState;

/// A query builder for projected column results.
#[derive(Clone)]
pub struct SelectQueryBuilder<T: IEntityType> {
    pub(crate) state: QueryState,
    pub(crate) provider: Option<Arc<dyn IDatabaseProvider>>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: IEntityType> SelectQueryBuilder<T> {
    /// Returns the generated SQL.
    pub fn to_sql(&self) -> String {
        let resolved = self.resolved_state();
        if let Some(provider) = &self.provider {
            let gen = provider.sql_generator();
            resolved.to_sql_with(gen)
        } else {
            resolved.to_sql()
        }
    }

    /// Returns the generated SQL and its bound parameters in SQL text order.
    ///
    /// Same contract as [`QueryBuilder::compile_sql`](super::QueryBuilder::compile_sql):
    /// parameters follow `all_params()` (CTE -> projections -> WHERE -> HAVING ->
    /// set operations). Useful for logging, SQL-shape assertions and custom
    /// execution wrappers; the terminals already bind the same vector.
    pub fn compile_sql(&self) -> (String, Vec<DbValue>) {
        let resolved = self.resolved_state();
        let sql = match &self.provider {
            Some(provider) => resolved.to_sql_with(provider.sql_generator()),
            None => resolved.to_sql(),
        };
        (sql, resolved.all_params())
    }

    /// Query state with correlated subqueries resolved from `T`'s navigation
    /// metadata (a projected query can still carry an `EXISTS`/`IN (SELECT ...)`
    /// predicate; without this resolution those subqueries would compile with
    /// empty table names).
    fn resolved_state(&self) -> QueryState {
        let mut state = self.state.clone();
        resolve_state_subqueries::<T>(&mut state);
        state
    }

    // -------------------------------------------------------------------
    // Clause methods (runtime report API)
    //
    // `group_by`/`select_item(s)`/`having`/`order_by(_desc)`/`take`/`skip`/
    // `distinct` also exist on `QueryBuilder`, so a clause chain may put
    // `group_by`/`having`/`order_by`/`take`/`skip`/`distinct` before *or*
    // after the `select` step.
    // -------------------------------------------------------------------

    /// Replaces the GROUP BY columns.
    pub fn group_by(mut self, columns: &[&str]) -> Self {
        self.state.group_bys = columns.iter().map(|c| c.to_string()).collect();
        self
    }

    /// Appends one typed SELECT-list projection.
    ///
    /// Mixing with [`select`](crate::query::QueryBuilder::select) is safe: the
    /// plain columns become `SelectItem::column` projections in their original
    /// order, so no column is silently dropped.
    pub fn select_item(mut self, item: SelectItem) -> Self {
        self.absorb_plain_columns();
        self.state.projections.push(item);
        self
    }

    /// Appends typed SELECT-list projections in the given order.
    ///
    /// Mixing with [`select`](crate::query::QueryBuilder::select) is safe (see
    /// [`select_item`](Self::select_item)); an empty list is a programming
    /// error and is rejected in debug builds.
    pub fn select_items(mut self, items: &[SelectItem]) -> Self {
        debug_assert!(
            !items.is_empty(),
            "select_items needs at least one projection item"
        );
        self.absorb_plain_columns();
        self.state.projections.extend(items.iter().cloned());
        self
    }

    /// Folds a plain `select(&[...])` column list into typed projections so a
    /// later `select_item(s)` call appends instead of discarding it.
    fn absorb_plain_columns(&mut self) {
        if let Some(columns) = self.state.projected_columns.take() {
            self.state.projections.extend(
                columns
                    .into_iter()
                    .map(SelectItem::column)
                    .collect::<Vec<_>>(),
            );
        }
    }

    /// Returns a reference to the accumulated query state.
    ///
    /// Together with [`SelectItem::expr`] / [`SelectItem::alias`] this makes a
    /// runtime report introspectable (for logging/OpenAPI adapters).
    pub fn state(&self) -> &QueryState {
        &self.state
    }

    /// Adds a HAVING condition (`AND`-joined with previous ones).
    pub fn having(mut self, expr: HavingExpr) -> Self {
        self.state.havings.push(expr);
        self
    }

    /// Adds an ascending ORDER BY column (may reference a projection alias).
    ///
    /// `column` is an SQL identifier / output alias and is emitted verbatim:
    /// use code constants or a hard-coded whitelist, never request data.
    pub fn order_by(mut self, column: &str) -> Self {
        super::expr::debug_assert_identifier(column, "ORDER BY column");
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Ascending));
        self
    }

    /// Adds a descending ORDER BY column (may reference a projection alias).
    ///
    /// `column` is an SQL identifier / output alias and is emitted verbatim:
    /// use code constants or a hard-coded whitelist, never request data.
    pub fn order_by_desc(mut self, column: &str) -> Self {
        super::expr::debug_assert_identifier(column, "ORDER BY column");
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Descending));
        self
    }

    /// Takes the specified number of rows.
    pub fn take(mut self, count: usize) -> Self {
        self.state.limit = Some(count);
        self
    }

    /// Skips the specified number of rows.
    pub fn skip(mut self, count: usize) -> Self {
        self.state.offset = Some(count);
        self
    }

    /// Marks this query as `SELECT DISTINCT`.
    pub fn distinct(mut self) -> Self {
        self.state.distinct = true;
        self
    }

    // `#[doc(hidden)]` aliases kept for the existing `linq!` expansion, which
    // emits these names regardless of clause order.

    /// `#[doc(hidden)]` — `linq!(group_by ...)` compatibility alias.
    #[doc(hidden)]
    pub fn group_by_internal(self, columns: &'static [&'static str]) -> Self {
        self.group_by(columns)
    }

    /// `#[doc(hidden)]` — `linq!(having ...)` compatibility alias.
    #[doc(hidden)]
    pub fn having_expr_internal(self, expr: HavingExpr) -> Self {
        self.having(expr)
    }

    /// `#[doc(hidden)]` — `linq!(order_by ...)` compatibility alias.
    #[doc(hidden)]
    pub fn order_by_column(self, column: &str) -> Self {
        self.order_by(column)
    }

    /// `#[doc(hidden)]` — `linq!(order_by descending ...)` compatibility alias.
    #[doc(hidden)]
    pub fn order_by_desc_column(self, column: &str) -> Self {
        self.order_by_desc(column)
    }

    /// Executes the projection query and returns raw column values per row.
    pub async fn to_list(self) -> EFResult<Vec<Vec<DbValue>>> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::configuration(
                "No provider attached to SelectQueryBuilder.".to_string(),
            )
        })?;
        let resolved = self.resolved_state();
        let gen = provider.sql_generator();
        let sql = resolved.to_sql_with(gen);
        let params = resolved.all_params();
        let mut conn = provider.get_connection().await?;
        conn.query(&sql, &params).await
    }

    /// Executes the projection query and materializes each row into `R` via
    /// [`IRow`].
    ///
    /// Use this with a `#[derive(FromRow)]` struct or a 1..=12-element tuple
    /// when the fixed-arity `to_list_typed_*` terminals are not enough. The
    /// mapping is positional: the `SELECT` list order must match the struct
    /// field / tuple element order.
    pub async fn to_list_rows<R: IRow>(self) -> EFResult<Vec<R>> {
        let rows = self.to_list().await?;
        crate::row::materialize_rows(&rows)
    }

    // -------------------------------------------------------------------
    // G3: Strongly-typed projection terminal methods.
    //
    // Each `to_list_typed_n::<V0, ...>` method executes the projection
    // query, then converts each column value via `TryFrom<DbValue>` into the
    // corresponding type parameter, returning `Vec<(V0, ...)>`.
    // -------------------------------------------------------------------

    async fn fetch_rows(self) -> EFResult<Vec<Vec<DbValue>>> {
        self.to_list().await
    }

    /// Single-column typed projection → `Vec<V0>`.
    pub async fn to_list_typed_1<V0>(self) -> EFResult<Vec<V0>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let cell = row
                    .first()
                    .ok_or_else(|| crate::error::EFError::query("projection row has no columns"))?;
                V0::try_from(cell.clone()).map_err(crate::error::EFError::from)
            })
            .collect()
    }

    /// Two-column typed projection → `Vec<(V0, V1)>`.
    pub async fn to_list_typed_2<V0, V1>(self) -> EFResult<Vec<(V0, V1)>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
        V1: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 0")
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 1")
                })?;
                Ok((
                    V0::try_from(c0.clone()).map_err(crate::error::EFError::from)?,
                    V1::try_from(c1.clone()).map_err(crate::error::EFError::from)?,
                ))
            })
            .collect()
    }

    /// Three-column typed projection → `Vec<(V0, V1, V2)>`.
    pub async fn to_list_typed_3<V0, V1, V2>(self) -> EFResult<Vec<(V0, V1, V2)>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
        V1: TryFrom<DbValue, Error = DbValueConvertError>,
        V2: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 0")
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 1")
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 2")
                })?;
                Ok((
                    V0::try_from(c0.clone()).map_err(crate::error::EFError::from)?,
                    V1::try_from(c1.clone()).map_err(crate::error::EFError::from)?,
                    V2::try_from(c2.clone()).map_err(crate::error::EFError::from)?,
                ))
            })
            .collect()
    }

    /// Four-column typed projection → `Vec<(V0, V1, V2, V3)>`.
    pub async fn to_list_typed_4<V0, V1, V2, V3>(self) -> EFResult<Vec<(V0, V1, V2, V3)>>
    where
        V0: TryFrom<DbValue, Error = DbValueConvertError>,
        V1: TryFrom<DbValue, Error = DbValueConvertError>,
        V2: TryFrom<DbValue, Error = DbValueConvertError>,
        V3: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 0")
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 1")
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 2")
                })?;
                let c3 = row.get(3).ok_or_else(|| {
                    crate::error::EFError::query("projection row missing column 3")
                })?;
                Ok((
                    V0::try_from(c0.clone()).map_err(crate::error::EFError::from)?,
                    V1::try_from(c1.clone()).map_err(crate::error::EFError::from)?,
                    V2::try_from(c2.clone()).map_err(crate::error::EFError::from)?,
                    V3::try_from(c3.clone()).map_err(crate::error::EFError::from)?,
                ))
            })
            .collect()
    }
}
