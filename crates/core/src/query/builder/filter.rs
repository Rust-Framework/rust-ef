//! Filter, ordering, distinct, subquery, and pagination methods.

use std::marker::PhantomData;

use crate::entity::IEntityType;
use crate::provider::DbValue;

use super::super::ast::{
    BoolExpr, FilterCondition, InSubquerySpec, OrderBy, OrderDirection, SubquerySpec,
};
use super::super::compile::{collect_bool_expr_values, filters_to_and_expr};
use super::super::state::QueryState;
use super::core::QueryBuilder;

impl<T: IEntityType> QueryBuilder<T> {
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
}
