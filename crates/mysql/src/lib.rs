//! MySQL provider for Rust Entity Framework.

pub mod connection;
pub mod di_extension;
pub mod introspection;
pub mod provider;
pub mod row_conversion;
pub mod sql_generator;
pub mod tls;
pub mod type_conversion;
pub mod type_mapping;

pub use connection::MySqlConnection;
pub use di_extension::DbContextOptionsBuilderExt;
pub use introspection::{introspect_mysql, DbColumn, DbTable};
pub use provider::MySqlProvider;
pub use sql_generator::MySqlSqlGenerator;
pub use tls::MySqlTlsMode;
pub use type_mapping::MySqlTypeMapping;
