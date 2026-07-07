//! Query source traits and column value parsing.
//!
//! `LinqSource` is a marker trait implemented by query source types
//! (`QueryBuilder<T>`, `DbSet<T>`) so the `linq!` macro can accept untyped
//! closures when the source carries entity type information.
//!
//! `ParseFromDb` parses `&str` column values from the database into Rust
//! types, handling database-specific representations (e.g. `"0"`/`"1"` for
//! booleans).
//!
//! `IQueryable` is the trait representing a queryable data source.

use crate::entity::IEntityType;
use crate::error::EFResult;

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

/// Parses a `&str` column value from the database into a Rust type.
///
/// Unlike `FromStr`, this trait handles database-specific representations:
/// - `bool`: accepts `"0"`/`"1"` (SQLite/MySQL) as well as `"true"`/`"false"`
/// - Numeric types: fall back to `FromStr`
/// - `String`: returns the value as-is
pub trait ParseFromDb: Sized {
    fn parse_from_db(s: &str) -> EFResult<Self>;
}

impl ParseFromDb for bool {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        match s {
            "1" | "true" | "t" | "TRUE" | "T" => Ok(true),
            "0" | "false" | "f" | "FALSE" | "F" | "" => Ok(false),
            _ => Err(crate::error::EFError::Query(format!(
                "failed to parse '{}' as bool",
                s
            ))),
        }
    }
}

impl ParseFromDb for i32 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as i32: {}", s, e))
        })
    }
}

impl ParseFromDb for i64 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as i64: {}", s, e))
        })
    }
}

impl ParseFromDb for f64 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as f64: {}", s, e))
        })
    }
}

impl ParseFromDb for f32 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as f32: {}", s, e))
        })
    }
}

impl ParseFromDb for String {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        Ok(s.to_string())
    }
}

/// Parses a `&str` column value into `V` via `ParseFromDb`.
pub(crate) fn parse_column<V: ParseFromDb>(s: &str) -> EFResult<V> {
    V::parse_from_db(s)
}

/// Trait representing a queryable data source.
pub trait IQueryable<T: IEntityType> {
    fn query(&self) -> QueryBuilder<T>;
}
