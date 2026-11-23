//! `ExecuteUpdateBuilder<T>` — bulk update operation builder.
//!
//! Produces `UPDATE <table> SET col = ?, ... WHERE <expr>` SQL with
//! parameters ordered as: SET values first, then WHERE parameters.
//!
//! 2.7 adds expression right-hand sides ([`ColumnExpr`]): a `SET` value can be
//! a column, a bound value, or column arithmetic (`col = col + ?`). That makes
//! atomic increments/decrements and compare-and-swap version bumps possible
//! without a read-modify-write round trip.

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::{DbValue, IDatabaseProvider, ISqlGenerator};

use super::compile::{compile_bool_expr, PortablePlaceholderGenerator};
use super::state::QueryState;

/// Expression used as the right-hand side of a `SET` assignment.
///
/// This mirrors only the `Column` / `Literal` / `Binary` subset of
/// [`SqlExpr`](crate::query::SqlExpr): a `SET` list accepts column arithmetic
/// but never aggregates / `COALESCE` / `CASE`, so the public surface stays
/// minimal instead of exposing the projection AST here.
///
/// Build values with the constructors ([`column`](Self::column),
/// [`value`](Self::value), [`add`](Self::add), [`sub`](Self::sub)); `DbValue`
/// converts with `From`.
#[derive(Debug, Clone)]
pub enum ColumnExpr {
    /// A bare column reference, e.g. `version` in `version = version + ?`.
    Column(String),
    /// A bound parameter value (`?`).
    Value(DbValue),
    /// `left + right`.
    Add(Box<ColumnExpr>, Box<ColumnExpr>),
    /// `left - right`.
    Sub(Box<ColumnExpr>, Box<ColumnExpr>),
}

impl ColumnExpr {
    /// A column reference (normally an entity `COLUMN_*` constant).
    ///
    /// The name is rendered as an SQL identifier verbatim: use entity
    /// `COLUMN_*` constants or a hard-coded whitelist, never request data.
    pub fn column(column: impl Into<String>) -> Self {
        let column = column.into();
        super::expr::debug_assert_identifier(&column, "ColumnExpr::column");
        ColumnExpr::Column(column)
    }

    /// A bound parameter value.
    pub fn value(value: impl Into<DbValue>) -> Self {
        ColumnExpr::Value(value.into())
    }

    /// `self + other`.
    ///
    /// A named builder method (not `std::ops::Add`) because the other operand
    /// is `impl Into<ColumnExpr>`, and because the expression is compiled to
    /// SQL rather than evaluated in Rust.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: impl Into<ColumnExpr>) -> Self {
        ColumnExpr::Add(Box::new(self), Box::new(other.into()))
    }

    /// `self - other`.
    #[allow(clippy::should_implement_trait)]
    pub fn sub(self, other: impl Into<ColumnExpr>) -> Self {
        ColumnExpr::Sub(Box::new(self), Box::new(other.into()))
    }

    /// Compiles the expression to SQL, advancing `param_idx` for each bound
    /// value in left-to-right order (the same order as
    /// [`collect_params`](Self::collect_params)).
    pub(crate) fn to_sql(&self, gen: &dyn ISqlGenerator, param_idx: &mut usize) -> String {
        match self {
            ColumnExpr::Column(column) => column.clone(),
            ColumnExpr::Value(_) => {
                let placeholder = gen.parameter_placeholder(*param_idx);
                *param_idx += 1;
                placeholder
            }
            ColumnExpr::Add(left, right) => {
                // `a + (b - c)` must keep its parentheses: without them SQL
                // evaluates `(a + b) - c`. `+` is associative on its own.
                let left_sql = left.to_sql(gen, param_idx);
                let right_sql = right.to_sql(gen, param_idx);
                if matches!(**right, ColumnExpr::Sub(..)) {
                    format!("{} + ({})", left_sql, right_sql)
                } else {
                    format!("{} + {}", left_sql, right_sql)
                }
            }
            ColumnExpr::Sub(left, right) => {
                // `a - (b + c)` / `a - (b - c)` need parentheses; a column or
                // value operand is already atomic.
                let left_sql = left.to_sql(gen, param_idx);
                let right_sql = right.to_sql(gen, param_idx);
                if matches!(**right, ColumnExpr::Add(..) | ColumnExpr::Sub(..)) {
                    format!("{} - ({})", left_sql, right_sql)
                } else {
                    format!("{} - {}", left_sql, right_sql)
                }
            }
        }
    }

    /// Returns the bound parameter values carried by this expression, in the
    /// same order as their placeholders in [`to_sql`](Self::to_sql).
    pub(crate) fn collect_params(&self) -> Vec<DbValue> {
        match self {
            ColumnExpr::Column(_) => Vec::new(),
            ColumnExpr::Value(value) => vec![value.clone()],
            ColumnExpr::Add(left, right) | ColumnExpr::Sub(left, right) => {
                let mut params = left.collect_params();
                params.extend(right.collect_params());
                params
            }
        }
    }
}

impl From<DbValue> for ColumnExpr {
    fn from(value: DbValue) -> Self {
        ColumnExpr::Value(value)
    }
}

/// Builder for bulk update operations.
#[derive(Clone)]
pub struct ExecuteUpdateBuilder<T: IEntityType> {
    pub(crate) state: QueryState,
    pub(crate) updates: Vec<(String, ColumnExpr)>,
    pub(crate) provider: Option<Arc<dyn IDatabaseProvider>>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: IEntityType> ExecuteUpdateBuilder<T> {
    /// Sets a named column to a value (`SET col = ?`).
    ///
    /// This is the public, semver-stable entry point for dynamic bulk updates;
    /// `linq!` expands `set b.views, 10; execute_update` to `set_column` calls.
    /// The column name is normally an entity `COLUMN_*` constant, and the
    /// value is anything convertible into [`DbValue`]. For expression
    /// assignments (`col = col + ?`, `col = other_col`) use
    /// [`set_column_expr`](Self::set_column_expr),
    /// [`increment`](Self::increment) or [`decrement`](Self::decrement).
    ///
    /// # Conditional update (compare-and-swap / optimistic concurrency)
    ///
    /// The `WHERE` guard and every `SET` assignment execute as **one**
    /// statement, so a version/status guard and the version bump cannot be
    /// interleaved with a concurrent writer. `execute()` returns the number of
    /// affected rows: `0` means the guard no longer matched (the row was
    /// changed or removed by someone else):
    ///
    /// ```ignore
    /// let affected = ctx
    ///     .set::<Order>()
    ///     .query()
    ///     .filter(linq!(|o: Order| o.id == order_id && o.version == expected_version))
    ///     .execute_update()
    ///     .set_column(Order::COLUMN_STATUS, "paid")
    ///     .increment(Order::COLUMN_VERSION, 1)
    ///     .execute()
    ///     .await?;
    ///
    /// if affected == 0 {
    ///     // lost the race: re-read the row and retry or report a conflict
    /// }
    /// ```
    pub fn set_column(mut self, column: &'static str, value: impl Into<DbValue>) -> Self {
        self.updates
            .push((column.to_string(), ColumnExpr::Value(value.into())));
        self
    }

    /// Sets a named column to a [`ColumnExpr`] (`SET col = <expr>`).
    ///
    /// Use this for assignments whose right-hand side is a column or column
    /// arithmetic; `increment`/`decrement` cover the common `col ± value`
    /// forms. All bound values are emitted in assignment order, before the
    /// `WHERE` parameters.
    ///
    /// ```ignore
    /// // SET stock = stock + reserved
    /// let affected = ctx
    ///     .set::<Product>()
    ///     .query()
    ///     .execute_update()
    ///     .set_column_expr(
    ///         Product::COLUMN_STOCK,
    ///         ColumnExpr::column(Product::COLUMN_STOCK)
    ///             .add(ColumnExpr::column(Product::COLUMN_RESERVED)),
    ///     )
    ///     .execute()
    ///     .await?;
    /// ```
    pub fn set_column_expr(mut self, column: &'static str, expr: ColumnExpr) -> Self {
        self.updates.push((column.to_string(), expr));
        self
    }

    /// Atomically increments a column: `SET col = col + ?`.
    ///
    /// `by` is bound as a parameter (never inlined), and the increment is part
    /// of the same statement as the `WHERE` guard — the natural way to bump a
    /// version column for optimistic concurrency, or to adjust a counter.
    ///
    /// ```ignore
    /// let affected = ctx
    ///     .set::<Order>()
    ///     .query()
    ///     .filter(linq!(|o: Order| o.id == order_id && o.version == expected_version))
    ///     .execute_update()
    ///     .increment(Order::COLUMN_VERSION, 1)
    ///     .execute()
    ///     .await?;
    /// ```
    pub fn increment(mut self, column: &'static str, by: impl Into<DbValue>) -> Self {
        self.updates.push((
            column.to_string(),
            ColumnExpr::column(column).add(ColumnExpr::value(by)),
        ));
        self
    }

    /// Atomically decrements a column: `SET col = col - ?`.
    ///
    /// See [`increment`](Self::increment) for the parameter and CAS semantics.
    pub fn decrement(mut self, column: &'static str, by: impl Into<DbValue>) -> Self {
        self.updates.push((
            column.to_string(),
            ColumnExpr::column(column).sub(ColumnExpr::value(by)),
        ));
        self
    }

    /// Deprecated alias for [`set_column`](Self::set_column), kept for one
    /// release so existing callers keep compiling.
    #[deprecated(
        since = "2.3.0",
        note = "renamed to `set_column`, which is now public and semver-stable"
    )]
    pub fn set_column_internal(self, column: &'static str, value: impl Into<DbValue>) -> Self {
        self.set_column(column, value)
    }

    /// Returns the generated SQL.
    ///
    /// SET expression parameters are emitted in assignment order, then the
    /// WHERE parameters; the same order [`params`](Self::params) returns.
    pub fn to_sql(&self) -> String {
        let gen: &dyn crate::provider::ISqlGenerator = self
            .provider
            .as_ref()
            .map(|p| p.sql_generator())
            .unwrap_or(&PortablePlaceholderGenerator);
        let mut param_idx = 1usize;
        let sets: Vec<String> = self
            .updates
            .iter()
            .map(|(column, expr)| format!("{} = {}", column, expr.to_sql(gen, &mut param_idx)))
            .collect();
        let where_clause = match &self.state.where_expr {
            Some(expr) => {
                let mut where_idx = param_idx;
                compile_bool_expr(expr, gen, &mut where_idx)
            }
            None => String::new(),
        };
        if where_clause.is_empty() {
            format!("UPDATE {} SET {}", self.state.from, sets.join(", "))
        } else {
            format!(
                "UPDATE {} SET {} WHERE {}",
                self.state.from,
                sets.join(", "),
                where_clause
            )
        }
    }

    /// Returns params for this bulk update: SET expression values first (in
    /// assignment order), then WHERE parameters.
    pub fn params(&self) -> Vec<DbValue> {
        let mut params = Vec::new();
        for (_, expr) in &self.updates {
            params.extend(expr.collect_params());
        }
        params.extend_from_slice(&self.state.parameters);
        params
    }

    /// Executes the bulk update.
    ///
    /// Returns [`EFError::configuration`](crate::error::EFError::configuration)
    /// when no `set` column was added — `UPDATE ... SET` without assignments is
    /// not valid SQL.
    pub async fn execute(self) -> EFResult<u64> {
        if self.updates.is_empty() {
            return Err(crate::error::EFError::configuration(
                "execute_update requires at least one set column (e.g. \
                 `set_column`)"
                    .to_string(),
            ));
        }
        let sql = self.to_sql();
        let params = self.params();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::configuration(
                "No provider attached to ExecuteUpdateBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        conn.execute(&sql, &params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::compile::PortablePlaceholderGenerator;

    fn sql(expr: &ColumnExpr) -> String {
        let mut idx = 1;
        expr.to_sql(&PortablePlaceholderGenerator, &mut idx)
    }

    #[test]
    fn nested_subtraction_keeps_its_parentheses() {
        // `a + (b - c)` must not collapse to `a + b - c`; the right operand of
        // `-` keeps parentheses when it is itself additive.
        let add_sub =
            ColumnExpr::column("a").add(ColumnExpr::column("b").sub(ColumnExpr::column("c")));
        assert_eq!(sql(&add_sub), "a + (b - c)");

        let sub_add =
            ColumnExpr::column("a").sub(ColumnExpr::column("b").add(ColumnExpr::column("c")));
        assert_eq!(sql(&sub_add), "a - (b + c)");

        // Associative shapes stay flat.
        let add_add =
            ColumnExpr::column("a").add(ColumnExpr::column("b").add(ColumnExpr::column("c")));
        assert_eq!(sql(&add_add), "a + b + c");

        let chained = ColumnExpr::column("a")
            .sub(ColumnExpr::column("b").add(ColumnExpr::column("c")))
            .sub(ColumnExpr::column("d"));
        assert_eq!(sql(&chained), "a - (b + c) - d");
    }

    #[test]
    fn values_are_bound_in_left_to_right_order() {
        let expr = ColumnExpr::column("a")
            .add(ColumnExpr::value(1i64))
            .sub(ColumnExpr::value(2i64));
        let mut idx = 1;
        assert_eq!(
            expr.to_sql(&PortablePlaceholderGenerator, &mut idx),
            "a + ? - ?"
        );
        assert_eq!(
            expr.collect_params(),
            vec![DbValue::I64(1), DbValue::I64(2)]
        );
        assert_eq!(idx, 3);
    }
}
