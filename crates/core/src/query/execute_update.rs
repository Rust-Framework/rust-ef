//! `ExecuteUpdateBuilder<T>` — bulk update operation builder.
//!
//! Produces `UPDATE <table> SET col = ?, ... WHERE <expr>` SQL with
//! parameters ordered as: SET values first, then WHERE parameters.

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::{DbValue, IDatabaseProvider};

use super::compile::{compile_bool_expr, PortablePlaceholderGenerator};
use super::state::QueryState;

/// Builder for bulk update operations.
#[derive(Clone)]
pub struct ExecuteUpdateBuilder<T: IEntityType> {
    pub(crate) state: QueryState,
    pub(crate) updates: Vec<(String, DbValue)>,
    pub(crate) provider: Option<Arc<dyn IDatabaseProvider>>,
    pub(crate) _phantom: PhantomData<T>,
}

impl<T: IEntityType> ExecuteUpdateBuilder<T> {
    /// Sets a named column to a value.
    ///
    /// This is the public, semver-stable entry point for dynamic bulk updates;
    /// `linq!` expands `set b.views, 10; execute_update` to `set_column` calls.
    /// The column name is normally an entity `COLUMN_*` constant, and the
    /// value is anything convertible into [`DbValue`].
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
    ///     .set_column(Order::COLUMN_VERSION, expected_version + 1)
    ///     .execute()
    ///     .await?;
    ///
    /// if affected == 0 {
    ///     // lost the race: re-read the row and retry or report a conflict
    /// }
    /// ```
    pub fn set_column(mut self, column: &'static str, value: impl Into<DbValue>) -> Self {
        self.updates.push((column.to_string(), value.into()));
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
            .map(|(col, _)| {
                let ph = gen.parameter_placeholder(param_idx);
                param_idx += 1;
                format!("{} = {}", col, ph)
            })
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

    /// Returns params for this bulk update.
    pub fn params(&self) -> Vec<DbValue> {
        let mut params: Vec<DbValue> = self.updates.iter().map(|(_, v)| v.clone()).collect();
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
