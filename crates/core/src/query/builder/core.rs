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
/// `Clone` is derived so that builders can be forked for compositional reuse
/// (e.g. applying additional filters on a base query without losing the
/// original). Note that `single`/`single_or_default` still use the `take(2)`
/// approach rather than `clone().count()` to avoid a double round-trip.
#[derive(Clone)]
pub struct QueryBuilder<T: IEntityType> {
    pub(super) state: QueryState,
    pub(super) provider: Option<Arc<dyn IDatabaseProvider>>,
    pub(super) filter_map: Option<Arc<HashMap<String, CompiledFilter>>>,
    pub(super) lazy_loading_enabled: bool,
    pub(super) _phantom: PhantomData<T>,
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

    /// Applies a global query filter `BoolExpr` (produced by `linq!(filter |b: T| ...)`).
    /// Inline values carried by the expression are collected and appended to
    /// the query parameters in the correct position.
    pub(crate) fn apply_query_filter(mut self, filter: BoolExpr) -> Self {
        let values = collect_bool_expr_values(&filter);
        self.state.parameters.extend(values);
        self.state.append_bool_expr(filter);
        self
    }
}
