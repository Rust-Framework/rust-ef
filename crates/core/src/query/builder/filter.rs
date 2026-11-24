//! Predicates, ordering, distinct, and pagination methods.

use crate::entity::IEntityType;

use super::super::ast::{qualify_bool_expr, OrderBy, OrderDirection};
use super::super::field::Field;
use super::super::predicate::Predicate;
use super::core::QueryBuilder;

impl<T: IEntityType> QueryBuilder<T> {
    /// Adds an ascending ORDER BY column.
    ///
    /// `column` is an SQL identifier / entity `COLUMN_*` constant and is emitted
    /// verbatim; never pass request data.
    pub fn order_by(mut self, column: &str) -> Self {
        super::super::expr::debug_assert_identifier(column, "ORDER BY column");
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Ascending));
        self
    }

    /// Adds a descending ORDER BY column.
    ///
    /// `column` is an SQL identifier / entity `COLUMN_*` constant and is emitted
    /// verbatim; never pass request data.
    pub fn order_by_desc(mut self, column: &str) -> Self {
        super::super::expr::debug_assert_identifier(column, "ORDER BY column");
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Descending));
        self
    }

    /// Adds an ascending ORDER BY from a typed [`Field`] handle.
    ///
    /// The handle is emitted by `#[derive(EntityType)]` (`Order::created_at`), so
    /// a typo or a renamed field is a compile error instead of a runtime SQL
    /// error. This is additive in 2.x; 3.0 makes it the only ordering entry.
    pub fn order_by_field<V>(self, field: Field<T, V>) -> Self {
        self.order_by(field.column())
    }

    /// Adds a descending ORDER BY from a typed [`Field`] handle.
    pub fn order_by_field_desc<V>(self, field: Field<T, V>) -> Self {
        self.order_by_desc(field.column())
    }

    /// Adds the next ascending ORDER BY column from a typed [`Field`] handle.
    ///
    /// Same semantics as [`Self::order_by_field`]; the name expresses "this is a
    /// tie-breaker after the previous ordering" (SQL has no separate clause).
    pub fn then_by_field<V>(self, field: Field<T, V>) -> Self {
        self.order_by_field(field)
    }

    /// Adds the next descending ORDER BY column from a typed [`Field`] handle.
    pub fn then_by_field_desc<V>(self, field: Field<T, V>) -> Self {
        self.order_by_field_desc(field)
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
    pub fn filter_joined<J: IEntityType>(mut self, predicate: impl Into<Predicate<J>>) -> Self {
        let table = J::entity_meta().table_name.to_string();
        match predicate.into().into_where_expr() {
            Some(expr) => {
                if super::super::compile::has_any_subquery(&expr) {
                    self.state.set_validation_error(
                        "filter_joined does not support EXISTS/IN subquery predicates: \
                         the subquery is resolved against the root entity, not the joined \
                         entity. Use a raw JOIN/filter on the root query instead",
                    );
                    return self;
                }
                self.apply_query_filter(qualify_bool_expr(expr, &table))
            }
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
