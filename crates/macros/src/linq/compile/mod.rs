//! Expression compilation for the `linq!` macro.
//!
//! Translates `syn::Expr` AST nodes into `TokenStream2` fragments that
//! construct either `QueryBuilder` method chains (Form A/B where body) or
//! `BoolExpr` value expressions (Form C filter, `&&`/`||` combinators).
//!
//! Module structure:
//! - `bool.rs` — Form C filter → `BoolExpr` value compilation
//! - `chain.rs` — Form A/B where body → `QueryBuilder` method chain + dispatch
//! - `subquery.rs` — EXISTS / IN (SELECT…) subquery compilation
//! - `having.rs` — `HavingExpr` value for `having` clauses

mod bool;
mod chain;
mod having;
mod subquery;

pub(crate) use bool::compile_bool_expr;
pub(crate) use chain::{compile_expr, compile_order};
pub(crate) use having::compile_having_expr;
