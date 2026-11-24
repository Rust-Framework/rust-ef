//! `QueryBuilder<T>` struct and core constructors.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::IEntityType;
use crate::provider::IDatabaseProvider;

use super::super::ast::{BoolExpr, CompiledFilter};
use super::super::compile::collect_bool_expr_values;
use super::super::state::QueryState;

/// A chainable query builder for entity type `T`.
///
/// Corresponds to EFCore's `IQueryable<T>`.
///
/// `Clone` is implemented manually (not derived) so forking a builder does not
/// require `T: Clone`: the phantom type parameter carries no data. This is what
/// lets `to_page` clone the query for its count without imposing an extra bound
/// on the entity. `single`/`single_or_default` still use the `take(2)` approach
/// rather than `clone().count()` to avoid a double round-trip.
pub struct QueryBuilder<T: IEntityType> {
    pub(super) state: QueryState,
    pub(super) provider: Option<Arc<dyn IDatabaseProvider>>,
    pub(super) filter_map: Option<Arc<HashMap<String, CompiledFilter>>>,
    pub(super) lazy_loading_enabled: bool,
    pub(super) _phantom: PhantomData<T>,
}

impl<T: IEntityType> Clone for QueryBuilder<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            provider: self.provider.clone(),
            filter_map: self.filter_map.clone(),
            lazy_loading_enabled: self.lazy_loading_enabled,
            _phantom: PhantomData,
        }
    }
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

    /// Applies a self-contained filter (a `BoolExpr` produced by
    /// `linq!(|b: T| ...)`, or a predicate's expression) to this query.
    ///
    /// Inline values stay inside the expression tree; the compiler emits their
    /// placeholders and collects the values in the same pass, so no separate
    /// parameter bookkeeping can drift.
    ///
    /// # Panics
    ///
    /// Panics when a hand-built condition declares a parameter count without
    /// carrying the values (`FilterCondition::new` with a non-zero count), which
    /// would otherwise reach the driver as a placeholder with no value.
    pub(crate) fn apply_query_filter(mut self, filter: BoolExpr) -> Self {
        let values = filter
            .total_param_count()
            .checked_sub(collect_bool_expr_values(&filter).len());
        assert_eq!(
            values,
            Some(0),
            "self-contained BoolExpr is missing bound values: build conditions with \
             `linq!(|b: T| ...)` or `FilterCondition::with_values`, not \
             `FilterCondition::new`"
        );
        self.state.append_bool_expr(filter);
        self
    }
}
