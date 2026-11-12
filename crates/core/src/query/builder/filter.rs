//! Predicates, ordering, distinct, and pagination methods.

use crate::entity::IEntityType;

use super::super::ast::{OrderBy, OrderDirection};
use super::super::predicate::Predicate;
use super::core::QueryBuilder;

impl<T: IEntityType> QueryBuilder<T> {
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

    /// Appends a predicate with `AND`.
    ///
    /// Accepts a [`Predicate`] — `linq!(|b: T| ...)` combined with
    /// [`Predicate::and`] / [`Predicate::or`] / [`Predicate::not`] — or a bare
    /// [`BoolExpr`](super::super::ast::BoolExpr). The predicate's bound values
    /// are registered in tree order, so they line up with the generated
    /// placeholders.
    ///
    /// Conditions accumulate with `AND`, and `Predicate::always()` adds no SQL,
    /// which makes it safe to seed a chain and apply it unconditionally:
    ///
    /// ```ignore
    /// let mut predicate = Predicate::<Product>::always();
    /// predicate = predicate.and_opt(
    ///     keyword.map(|keyword| linq!(|p: Product| p.name.contains(keyword))),
    /// );
    /// predicate = predicate.and_opt(
    ///     min_price.map(|min_price| linq!(|p: Product| p.price >= min_price)),
    /// );
    /// let query = ctx.set::<Product>().query().filter(predicate);
    /// ```
    pub fn filter(self, predicate: impl Into<Predicate<T>>) -> Self {
        match predicate.into().into_where_expr() {
            Some(expr) => self.apply_query_filter(expr),
            None => self,
        }
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
