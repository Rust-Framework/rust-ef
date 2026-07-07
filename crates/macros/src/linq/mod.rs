//! `linq!()` — compile-time LINQ-to-SQL.
//!
//! Single DSL entry point for all database operations. Supports three
//! syntactic forms: filter closures, multi-clause queries, and value-producing
//! configurations. See `linq!` macro docs in `lib.rs` for usage examples.

mod ast;
mod compile;
mod context;
mod expand;
mod parse;

pub use expand::expand_linq;
