//! `ExecuteUpdateBuilder<T>` — bulk update operation builder.
//!
//! Produces `UPDATE <table> SET col = ?, ... WHERE <expr>` SQL with
//! parameters ordered as: SET values first, then WHERE parameters.
//!
//! 2.9 adds expression right-hand sides ([`ColumnExpr`]): a `SET` value can be
//! a column, a bound value, or column arithmetic (`col = col + ?`). That makes
//! atomic increments/decrements and compare-and-swap version bumps possible
//! without a read-modify-write round trip.

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::{DbValue, IDatabaseProvider, ISqlGenerator};

use super::compile::{compile_bool_expr_into, PortablePlaceholderGenerator};
use super::expr::{ColumnExpr, SqlExpr};
use super::state::QueryState;

/// Builder for bulk update operations.
#[derive(Clone)]
pub struct ExecuteUpdateBuilder<T: IEntityType> {
    pub(crate) state: QueryState,
    pub(crate) updates: Vec<(String, SqlExpr)>,
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
            .push((column.to_string(), SqlExpr::literal(value.into())));
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
            SqlExpr::column(column).add(SqlExpr::literal(by)),
        ));
        self
    }

    /// Atomically decrements a column: `SET col = col - ?`.
    ///
    /// See [`increment`](Self::increment) for the parameter and CAS semantics.
    pub fn decrement(mut self, column: &'static str, by: impl Into<DbValue>) -> Self {
        self.updates.push((
            column.to_string(),
            SqlExpr::column(column).sub(SqlExpr::literal(by)),
        ));
        self
    }

    /// Returns the generated SQL (unchecked) for logging/tests.
    ///
    /// Use [`compile_sql`](Self::compile_sql) when the parameters are needed or
    /// the state must be validated.
    pub fn to_sql(&self) -> String {
        let gen: &dyn ISqlGenerator = self
            .provider
            .as_ref()
            .map(|p| p.sql_generator())
            .unwrap_or(&PortablePlaceholderGenerator);
        let mut discarded = Vec::new();
        self.compile_into(gen, &mut discarded).0
    }

    /// Returns the generated SQL and its bound parameters in one pass:
    /// SET expression values in assignment order, then WHERE parameters.
    pub fn compile_sql(&self) -> EFResult<(String, Vec<DbValue>)> {
        self.state.validate_for_dml()?;
        if self.updates.is_empty() {
            return Err(crate::error::EFError::configuration(
                "execute_update requires at least one set column (e.g. \
                 `set_column`, `set_column_expr`, `increment`)"
                    .to_string(),
            ));
        }
        for (column, expr) in &self.updates {
            super::expr::validate_set_expression(expr).map_err(|error| {
                crate::error::EFError::configuration(format!(
                    "invalid SET expression for column `{column}`: {error}"
                ))
            })?;
        }
        let gen: &dyn ISqlGenerator = self
            .provider
            .as_ref()
            .map(|p| p.sql_generator())
            .unwrap_or(&PortablePlaceholderGenerator);
        let mut params = Vec::new();
        let (sql, _) = self.compile_into(gen, &mut params);
        Ok((sql, params))
    }

    /// Returns params for this bulk update, validating the query state.
    pub fn params(&self) -> EFResult<Vec<DbValue>> {
        self.compile_sql().map(|(_, params)| params)
    }

    fn compile_into(&self, gen: &dyn ISqlGenerator, params: &mut Vec<DbValue>) -> (String, ()) {
        let mut param_idx = 1usize;
        let sets: Vec<String> = self
            .updates
            .iter()
            .map(|(column, expr)| {
                format!("{} = {}", column, expr.compile(gen, &mut param_idx, params))
            })
            .collect();
        let where_clause = match &self.state.where_expr {
            Some(expr) => compile_bool_expr_into(expr, gen, &mut param_idx, params),
            None => String::new(),
        };
        let sql = if where_clause.is_empty() {
            format!("UPDATE {} SET {}", self.state.from, sets.join(", "))
        } else {
            format!(
                "UPDATE {} SET {} WHERE {}",
                self.state.from,
                sets.join(", "),
                where_clause
            )
        };
        (sql, ())
    }

    /// Executes the bulk update.
    ///
    /// Returns [`EFError::configuration`](crate::error::EFError::configuration)
    /// when no `set` column was added — `UPDATE ... SET` without assignments is
    /// not valid SQL.
    pub async fn execute(self) -> EFResult<u64> {
        let (sql, params) = self.compile_sql()?;
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

    fn compile(expr: &ColumnExpr) -> (String, Vec<DbValue>) {
        let mut idx = 1;
        let mut params = Vec::new();
        let sql = expr.compile(&PortablePlaceholderGenerator, &mut idx, &mut params);
        (sql, params)
    }

    #[test]
    fn nested_expressions_keep_their_parentheses() {
        // The shared SqlExpr compiler parenthesises every binary node, so the
        // SET expression cannot be re-associated by SQL precedence.
        let add_sub =
            ColumnExpr::column("a").add(ColumnExpr::column("b").sub(ColumnExpr::column("c")));
        assert_eq!(compile(&add_sub).0, "(a + (b - c))");

        let sub_add =
            ColumnExpr::column("a").sub(ColumnExpr::column("b").add(ColumnExpr::column("c")));
        assert_eq!(compile(&sub_add).0, "(a - (b + c))");

        let add_add =
            ColumnExpr::column("a").add(ColumnExpr::column("b").add(ColumnExpr::column("c")));
        assert_eq!(compile(&add_add).0, "(a + (b + c))");

        let chained = ColumnExpr::column("a")
            .sub(ColumnExpr::column("b").add(ColumnExpr::column("c")))
            .sub(ColumnExpr::column("d"));
        assert_eq!(compile(&chained).0, "((a - (b + c)) - d)");
    }

    #[test]
    fn values_are_bound_in_left_to_right_order() {
        let expr = ColumnExpr::column("a")
            .add(ColumnExpr::value(1i64))
            .sub(ColumnExpr::value(2i64));
        let (sql, params) = compile(&expr);
        assert_eq!(sql, "((a + ?) - ?)");
        assert_eq!(params, vec![DbValue::I64(1), DbValue::I64(2)]);
    }
}
