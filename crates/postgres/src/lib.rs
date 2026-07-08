//! PostgreSQL provider for Rust Entity Framework.

pub mod connection;
pub mod di_extension;
pub mod introspection;
pub mod provider;
pub mod row_conversion;
pub mod sql_generator;
pub mod tls;
pub mod type_conversion;
pub mod type_mapping;

pub use connection::PostgresConnection;
pub use di_extension::DbContextOptionsBuilderExt;
pub use introspection::{introspect_postgres, DbColumn, DbTable};
pub use provider::PostgresProvider;
pub use sql_generator::PostgresSqlGenerator;
pub use tls::PgTlsMode;
pub use type_mapping::PostgresTypeMapping;
