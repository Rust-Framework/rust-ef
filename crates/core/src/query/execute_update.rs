//! `ExecuteUpdateBuilder<T>` — bulk update operation builder.
//!
//! Produces `UPDATE <table> SET col = ?, ... WHERE <expr>` SQL with
//! parameters ordered as: SET values first, then WHERE parameters.

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::error::EFResult;
use crate::provider::{DbValue, IDatabaseProvider};

use super::compile::{
    build_where_clause_with_offset, compile_bool_expr, PortablePlaceholderGenerator,
};
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
    /// Sets a named column to a DbValue.
    ///
    /// `#[doc(hidden)]` — called by `linq!(set b.views, 10; execute_update)`
    /// expansion.
    #[doc(hidden)]
    pub fn set_column_internal(mut self, column: &'static str, value: impl Into<DbValue>) -> Self {
        self.updates.push((column.to_string(), value.into()));
        self
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
        let where_clause = if let Some(ref expr) = self.state.where_expr {
            let mut param_idx = param_idx;
            compile_bool_expr(expr, gen, &mut param_idx)
        } else {
            build_where_clause_with_offset(&self.state.filters, gen, param_idx)
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
    pub async fn execute(self) -> EFResult<u64> {
        let sql = self.to_sql();
        let params = self.params();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to ExecuteUpdateBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        conn.execute(&sql, &params).await
    }
}
