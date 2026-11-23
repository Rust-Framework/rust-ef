//! Predicates, ordering, distinct, and pagination methods.

use crate::entity::IEntityType;

use super::super::ast::{qualify_bool_expr, OrderBy, OrderDirection};
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

    /// Appends a predicate written for the joined entity `J`, qualifying every
    /// plain filter column with `J`'s table name.
    ///
    /// A joined table is only reachable through its table qualifier: a
    /// `linq!(|p: Post| ...)` predicate produces unqualified columns like
    /// `title`, which either fail as ambiguous or silently filter the root
    /// table. `filter_joined` rewrites them to `posts.title`:
    ///
    /// ```ignore
    /// // ... JOIN posts ON blogs.id = posts.blog_id WHERE posts.published = ?
    /// let blogs = ctx
    ///     .set::<Blog>()
    ///     .query()
    ///     .join::<Post>(JoinKind::Inner, &[("id", "blog_id")])
    ///     .filter_joined(linq!(|p: Post| p.published))
    ///     .to_list()
    ///     .await?;
    /// ```
    ///
    /// Only plain filter columns are qualified: `EXISTS` / `IN (SELECT ...)`
    /// subquery nodes and raw fragments keep their own table scope, and a
    /// column that already carries a `.` qualifier is left untouched.
    pub fn filter_joined<J: IEntityType>(self, predicate: impl Into<Predicate<J>>) -> Self {
        let table = J::entity_meta().table_name.to_string();
        match predicate.into().into_where_expr() {
            Some(expr) => self.apply_query_filter(qualify_bool_expr(expr, &table)),
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
