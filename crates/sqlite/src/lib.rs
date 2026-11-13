//! SQLite provider for Rust Entity Framework.

#![forbid(unsafe_code)]

pub mod connection;
pub mod di_extension;
pub mod introspection;
pub mod pool_strategy;
pub mod provider;
pub mod sql_generator;
pub mod sync_ops;
pub mod type_conversion;

pub use connection::SqliteConnection;
pub use di_extension::DbContextOptionsBuilderExt;
#[allow(deprecated)]
// compatibility re-export of the legacy free function; prefer IDatabaseProvider::introspect
pub use introspection::introspect_sqlite;
pub use provider::{SqliteOptions, SqliteProvider};
pub use sql_generator::SqliteSqlGenerator;
