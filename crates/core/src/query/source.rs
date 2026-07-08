//! Query source traits.
//!
//! `LinqSource` is a marker trait implemented by query source types
//! (`QueryBuilder<T>`, `DbSet<T>`) so the `linq!` macro can accept untyped
//! closures when the source carries entity type information.
//!
//! `IQueryable` is the trait representing a queryable data source.

use crate::entity::IEntityType;

use super::builder::QueryBuilder;

/// Marker trait implemented by query source types (`QueryBuilder<T>`, `DbSet<T>`).
///
/// `LinqSource` enables the `linq!` macro to accept untyped closures
/// (`|b| ...`) when the source expression carries entity type information
/// via turbofish (e.g. `ctx.set::<Blog>()`). The macro extracts the type
/// from the source and generates a typed closure internally.
pub trait LinqSource {}

impl<T: IEntityType> LinqSource for QueryBuilder<T> {}
impl<T: IEntityType> LinqSource for crate::db_set::DbSet<T> {}

/// Trait representing a queryable data source.
pub trait IQueryable<T: IEntityType> {
    fn query(&self) -> QueryBuilder<T>;
}
