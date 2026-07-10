//! `syn::Parse` implementations for the `linq!` macro input grammar.
//!
//! Module structure:
//! - `input.rs` — `LinqInput` dispatch (Forms A/B/C) and query parsing
//! - `clauses.rs` — `LinqClause` dispatch and simple clause parsers
//! - `complex.rs` — window, CTE/with, and having AST conversion

mod clauses;
mod complex;
mod input;
