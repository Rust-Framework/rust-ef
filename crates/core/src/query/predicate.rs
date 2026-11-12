//! Composable query predicates — `AND` / `OR` / `NOT` over expression trees.
//!
//! `linq!(filter |b: T| ...)` produces a self-contained [`BoolExpr`] fragment
//! that already carries its bound values. [`Predicate<T>`] tags such a fragment
//! with the entity it was written for and adds the combinators needed to build
//! a WHERE clause condition by condition — the rust-ef equivalent of EF Core's
//! expression-tree `PredicateBuilder`:
//!
//! - [`Predicate::always`] / [`Predicate::never`] seed an `AND` / `OR` chain,
//! - [`Predicate::and`] / [`Predicate::or`] / [`Predicate::not`] nest freely,
//! - [`QueryBuilder::where_`] applies the result, appending it with `AND` and
//!   registering its values in tree order.
//!
//! Unlike a `linq!(|b: T| ...)` closure — which is a one-shot chain — a
//! predicate is a value: it can be built condition by condition, stored,
//! reused across queries, and it keeps its `OR` groups and bound values in
//! place when it is applied later.

use std::marker::PhantomData;

use crate::entity::IEntityType;

use super::ast::BoolExpr;

/// A composable WHERE predicate for entity type `T`.
///
/// `T` tags the predicate so that fragments written for different entities do
/// not silently combine or get applied to the wrong query builder.
///
/// ```
/// use rust_ef::prelude::*;
/// use rust_ef::query::QueryBuilder;
///
/// #[derive(Debug, Clone, EntityType)]
/// #[table("products")]
/// struct Product {
///     #[primary_key]
///     id: i32,
///     name: String,
///     description: String,
///     price: f64,
///     is_deleted: bool,
/// }
///
/// fn product_filter(keyword: Option<&str>, min_price: Option<f64>) -> Predicate<Product> {
///     // AND 链的起始条件；always() 本身不产生 SQL。
///     let mut predicate = Predicate::<Product>::always()
///         .and(linq!(filter |p: Product| !p.is_deleted));
///
///     // Name 或 Description 命中关键字：OR 组作为一个整体与外部 AND。
///     if let Some(keyword) = keyword {
///         predicate = predicate.and(
///             linq!(filter |p: Product| p.name.contains(keyword))
///                 .or(linq!(filter |p: Product| p.description.contains(keyword))),
///         );
///     }
///
///     if let Some(min_price) = min_price {
///         predicate = predicate.and(linq!(filter |p: Product| p.price >= min_price));
///     }
///     predicate
/// }
///
/// let predicate = product_filter(Some("rust"), Some(9.9));
/// let sql = QueryBuilder::<Product>::new("products")
///     .where_(predicate)
///     .to_sql();
/// // (is_deleted = ?) AND ((name LIKE ?) OR (description LIKE ?)) AND (price >= ?)
/// assert!(sql.contains("(name LIKE ?) OR (description LIKE ?)"));
/// ```
#[derive(Debug, Clone)]
pub struct Predicate<T: IEntityType> {
    kind: PredicateKind,
    _marker: PhantomData<fn() -> T>,
}

#[derive(Debug, Clone)]
enum PredicateKind {
    /// Matches every row (`1=1`) — the identity of `AND`.
    Always,
    /// Matches no row (`1=0`) — the identity of `OR`.
    Never,
    /// A self-contained expression tree, e.g. from `linq!(filter |b: T| ...)`.
    Expr(BoolExpr),
}

impl<T: IEntityType> Predicate<T> {
    /// A predicate that matches every row — the seed of an `AND` chain.
    ///
    /// `Predicate::always().and(p)` is `p`, so seeding with it adds no SQL.
    pub fn always() -> Self {
        Self::with_kind(PredicateKind::Always)
    }

    /// A predicate that matches no row — the seed of an `OR` chain.
    ///
    /// `Predicate::never().or(p)` is `p`, so seeding with it adds no SQL.
    pub fn never() -> Self {
        Self::with_kind(PredicateKind::Never)
    }

    /// Wraps an expression tree produced by `linq!(filter |b: T| ...)`.
    pub fn from_expr(expr: BoolExpr) -> Self {
        Self::with_kind(PredicateKind::Expr(expr))
    }

    fn with_kind(kind: PredicateKind) -> Self {
        Self {
            kind,
            _marker: PhantomData,
        }
    }

    /// `self AND other`.
    ///
    /// `always()` is the identity (`always().and(p) == p`) and `never()` is
    /// absorbing (`never().and(p) == never()`), so identities never leak into
    /// the generated SQL.
    pub fn and(self, other: impl Into<Predicate<T>>) -> Self {
        let rhs = other.into().kind;
        let kind = match (self.kind, rhs) {
            (PredicateKind::Never, _) | (_, PredicateKind::Never) => PredicateKind::Never,
            (PredicateKind::Always, rhs) => rhs,
            (lhs, PredicateKind::Always) => lhs,
            (PredicateKind::Expr(lhs), PredicateKind::Expr(rhs)) => {
                PredicateKind::Expr(BoolExpr::And(Box::new(lhs), Box::new(rhs)))
            }
        };
        Self::with_kind(kind)
    }

    /// `self OR other`.
    ///
    /// `never()` is the identity (`never().or(p) == p`) and `always()` is
    /// absorbing (`always().or(p) == always()`).
    pub fn or(self, other: impl Into<Predicate<T>>) -> Self {
        let rhs = other.into().kind;
        let kind = match (self.kind, rhs) {
            (PredicateKind::Always, _) | (_, PredicateKind::Always) => PredicateKind::Always,
            (PredicateKind::Never, rhs) => rhs,
            (lhs, PredicateKind::Never) => lhs,
            (PredicateKind::Expr(lhs), PredicateKind::Expr(rhs)) => {
                PredicateKind::Expr(BoolExpr::Or(Box::new(lhs), Box::new(rhs)))
            }
        };
        Self::with_kind(kind)
    }

    /// `NOT self`.
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        let kind = match self.kind {
            PredicateKind::Always => PredicateKind::Never,
            PredicateKind::Never => PredicateKind::Always,
            PredicateKind::Expr(expr) => PredicateKind::Expr(BoolExpr::Not(Box::new(expr))),
        };
        Self::with_kind(kind)
    }

    /// Converts the predicate to a [`BoolExpr`] — for APIs that take an
    /// expression directly, such as `ModelBuilder::has_query_filter`.
    ///
    /// `always()` and `never()` become the literal fragments `1=1` / `1=0`.
    pub fn into_expr(self) -> BoolExpr {
        match self.kind {
            PredicateKind::Always => BoolExpr::raw("1=1"),
            PredicateKind::Never => BoolExpr::raw("1=0"),
            PredicateKind::Expr(expr) => expr,
        }
    }

    /// WHERE-clause view: `None` for the `AND` identity (`always()`), which
    /// needs no SQL at all.
    pub(crate) fn into_where_expr(self) -> Option<BoolExpr> {
        match self.kind {
            PredicateKind::Always => None,
            PredicateKind::Never => Some(BoolExpr::raw("1=0")),
            PredicateKind::Expr(expr) => Some(expr),
        }
    }
}

impl<T: IEntityType> From<BoolExpr> for Predicate<T> {
    /// Lets `linq!(filter |b: T| ...)` fragments be combined directly.
    fn from(expr: BoolExpr) -> Self {
        Self::from_expr(expr)
    }
}

impl<T: IEntityType> From<Predicate<T>> for BoolExpr {
    fn from(predicate: Predicate<T>) -> Self {
        predicate.into_expr()
    }
}
