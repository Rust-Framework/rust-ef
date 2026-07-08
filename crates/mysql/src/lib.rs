//! MySQL provider for Rust Entity Framework.

pub mod connection;
pub mod di_extension;
pub mod introspection;
pub mod provider;
pub mod sql_generator;
pub mod type_conversion;
pub mod type_mapping;

pub use connection::MySqlConnection;
pub use di_extension::DbContextOptionsBuilderExt;
pub use introspection::{introspect_mysql, DbColumn, DbTable};
pub use provider::{MySqlProvider, MySqlTlsMode};
pub use sql_generator::MySqlSqlGenerator;
pub use type_mapping::MySqlTypeMapping;
