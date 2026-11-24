//! `QueryBuilder<T>` — chainable query builder for entity type `T`.
//!
//! Corresponds to EFCore's `IQueryable<T>`. The struct and core constructors
//! live in `core.rs`; filter/ordering/pagination methods in `filter.rs`;
//! include/JOIN methods in `join.rs`; GROUP BY/HAVING/window/CTE/set-op/
//! aggregate/projection methods in `aggregate.rs`; SQL compilation / terminal
//! execution methods in `terminal.rs`; the offset-pagination terminal
//! (`to_page`) in `page.rs`.

mod aggregate;
mod core;
mod filter;
mod join;
mod page;
mod terminal;

pub use core::QueryBuilder;
