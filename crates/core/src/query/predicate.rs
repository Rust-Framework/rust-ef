//! Composable query predicates — `AND` / `OR` / `NOT` over expression trees.
//!
//! `linq!(filter |b: T| ...)` produces a self-contained [`Predicate<T>`] that
//! already carries its bound values, so predicates can be built condition by
//! condition and combined before they are applied — the rust-ef equivalent of
//! EF Core's expression-tree `PredicateBuilder`:
//!
//! - [`Predicate::always`] / [`Predicate::never`] seed an `AND` / `OR` chain,
//! - [`Predicate::and`] / [`Predicate::or`] / [`Predicate::not`] nest freely,
//!   with [`Predicate::and_opt`] / [`Predicate::or_opt`] for optional filters,
//! - [`QueryBuilder::where_`] applies the result, appending it with `AND` and
//!   registering its values in tree order.
//!
//! A predicate is a value, not a one-shot `linq!(|b: T| ...)` closure: it can be
//! stored, cloned, and reused across the `count()` / page / export queries of
//! the same request. [`BoolExpr`] is the encoded tree behind it; use
//! [`Predicate::into_expr`] when an API asks for a bare expression.

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
///     let keyword_group = keyword.map(|keyword| {
///         linq!(filter |p: Product| p.name.contains(keyword))
///             .or(linq!(filter |p: Product| p.description.contains(keyword)))
///     });
///     let min_price = min_price.map(|min_price| linq!(filter |p: Product| p.price >= min_price));
///
///     // always() 是 AND 链起点，本身不产生 SQL；
///     // and_opt 让「可选条件」不需要额外 if let。
///     Predicate::<Product>::always()
///         .and(linq!(filter |p: Product| !p.is_deleted))
///         .and_opt(keyword_group)
///         .and_opt(min_price)
/// }
///
/// let predicate = product_filter(Some("rust"), Some(9.9));
/// let sql = QueryBuilder::<Product>::new("products")
///     .where_(predicate)
///     .to_sql();
/// // NOT (is_deleted = ?) AND ((name LIKE ?) OR (description LIKE ?)) AND (price >= ?)
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

    /// Wraps an expression tree as a predicate.
    ///
    /// Prefer `linq!(filter |b: T| ...)`, which is already typed and
    /// self-contained. This is the unchecked door for reusing a `BoolExpr`
    /// produced elsewhere (e.g. a filter read back from `ModelBuilder`).
    pub fn from_expr(expr: BoolExpr) -> Self {
        Self::with_kind(PredicateKind::Expr(expr))
    }

    fn with_kind(kind: PredicateKind) -> Self {
        Self {
            kind,
            _marker: PhantomData,
        }
    }

    /// Whether the predicate matches every row (the `AND` identity).
    pub fn is_always(&self) -> bool {
        matches!(self.kind, PredicateKind::Always)
    }

    /// Whether the predicate matches no row.
    pub fn is_never(&self) -> bool {
        matches!(self.kind, PredicateKind::Never)
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

    /// `self AND other` when `other` is `Some` — the one-liner for an optional
    /// filter. `None` leaves the predicate unchanged.
    pub fn and_opt(self, other: Option<impl Into<Predicate<T>>>) -> Self {
        match other {
            Some(other) => self.and(other),
            None => self,
        }
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

    /// `self OR other` when `other` is `Some`; `None` leaves the predicate
    /// unchanged.
    pub fn or_opt(self, other: Option<impl Into<Predicate<T>>>) -> Self {
        match other {
            Some(other) => self.or(other),
            None => self,
        }
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
    /// Lets `BoolExpr` fragments (e.g. a stored global filter) be combined
    /// directly. This conversion is unchecked: prefer `linq!(filter ...)`.
    fn from(expr: BoolExpr) -> Self {
        Self::from_expr(expr)
    }
}

impl<T: IEntityType> From<Predicate<T>> for BoolExpr {
    fn from(predicate: Predicate<T>) -> Self {
        predicate.into_expr()
    }
}
