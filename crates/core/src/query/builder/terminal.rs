//! SQL compilation and terminal execution methods (to_list, first, count, etc.).

use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::{
    IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, ILazyInit, INavigationSetter,
};
use crate::error::EFResult;
use crate::provider::{DbValue, IDatabaseProvider};

use super::super::ast::{OrderBy, OrderDirection};
use super::super::compile::{
    build_where_clauses, compile_bool_expr, has_subqueries, resolve_subqueries,
};
use super::super::execute_update::ExecuteUpdateBuilder;
use super::super::state::QueryState;
use super::core::QueryBuilder;

impl<T: IEntityType> QueryBuilder<T> {
    // -------------------------------------------------------------------
    // Find / Exists
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
                crate::error::EFError::query(format!(
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
                crate::error::EFError::query(format!(
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

    // -------------------------------------------------------------------
    // SQL compilation
    // -------------------------------------------------------------------

    /// Builds the SQL string for this query.
    pub fn to_sql(&self) -> String {
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

    pub(super) fn compile_state_sql(
        state: &QueryState,
        provider: &Arc<dyn IDatabaseProvider>,
    ) -> String {
        let gen = provider.sql_generator();
        let mut resolved = state.clone();
        if let Some(ref mut expr) = resolved.where_expr {
            if has_subqueries(expr) {
                let meta = T::entity_meta();
                resolve_subqueries(expr, &meta);
            }
        }
        resolved.to_sql_with(gen)
    }

    // -------------------------------------------------------------------
    // Terminal methods
    // -------------------------------------------------------------------

    /// Executes the query and returns all matching entities.
    pub async fn to_list(self) -> EFResult<Vec<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let includes = self.state.includes.clone();
        let lazy_loading = self.lazy_loading_enabled;
        let (sql, params) = self.compile_sql();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::configuration(
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
            .ok_or_else(|| crate::error::EFError::not_found("Entity not found".to_string()))
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
            crate::error::EFError::configuration(
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
                    crate::error::EFError::type_conversion(format!(
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
            crate::error::EFError::configuration(
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
            .ok_or_else(|| crate::error::EFError::not_found("Entity not found".to_string()))
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
                    crate::error::EFError::query(format!(
                        "last_or_default requires a primary key on {} when no explicit ordering is set",
                        std::any::type_name::<T>()
                    ))
                })?;
            self.state
                .orderings
                .push(OrderBy::new(pk_col.to_string(), OrderDirection::Descending));
        } else {
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
            return Err(crate::error::EFError::query(
                "Sequence contains more than one element".to_string(),
            ));
        }
        results.pop().ok_or_else(|| {
            crate::error::EFError::not_found("Sequence contains no elements".to_string())
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
            return Err(crate::error::EFError::query(
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
            crate::error::EFError::configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let gen = provider.sql_generator();
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
