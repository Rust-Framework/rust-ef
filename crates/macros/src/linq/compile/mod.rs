//! Expression compilation for the `linq!` macro.
//!
//! Translates `syn::Expr` AST nodes into `TokenStream2` fragments that build a
//! `BoolExpr` condition tree (compiled by the core crate into SQL) or a
//! `QueryBuilder` method chain (clauses and ordering).
//!
//! Module structure:
//! - `bool.rs` — condition expression → `BoolExpr` value compilation
//! - `chain.rs` — `=> field` ordering sugar
//! - `subquery.rs` — EXISTS / IN (SELECT…) subquery compilation
//! - `having.rs` — `HavingExpr` value for `having` clauses

mod bool;
mod chain;
mod having;
mod subquery;

pub(crate) use bool::compile_bool_expr;
pub(crate) use chain::compile_order;
pub(crate) use having::compile_having_expr;
