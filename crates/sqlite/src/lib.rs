//! SQLite provider for Rust Entity Framework.

pub mod connection;
pub mod di_extension;
pub mod pool_strategy;
pub mod provider;
pub mod sql_generator;
pub mod sync_ops;
pub mod type_conversion;

pub use connection::SqliteConnection;
pub use di_extension::DbContextOptionsBuilderExt;
pub use provider::SqliteProvider;
pub use sql_generator::SqliteSqlGenerator;
