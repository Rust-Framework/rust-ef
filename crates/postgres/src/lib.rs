//! PostgreSQL provider for Rust Entity Framework.

pub mod connection;
pub mod di_extension;
pub mod introspection;
pub mod provider;
pub mod sql_generator;
pub mod type_conversion;
pub mod type_mapping;

pub use connection::PostgresConnection;
pub use di_extension::DbContextOptionsBuilderExt;
pub use introspection::{introspect_postgres, DbColumn, DbTable};
pub use provider::{PgTlsMode, PostgresProvider};
pub use sql_generator::PostgresSqlGenerator;
pub use type_mapping::PostgresTypeMapping;
