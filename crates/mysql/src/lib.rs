//! MySQL provider for Rust Entity Framework.

#![forbid(unsafe_code)]

pub mod connection;
pub mod di_extension;
mod error_map;
pub mod introspection;
pub mod provider;
pub mod row_conversion;
pub mod sql_generator;
pub mod tls;
pub mod type_conversion;
pub mod type_mapping;

pub use connection::MySqlConnection;
pub use di_extension::DbContextOptionsBuilderExt;
#[allow(deprecated)]
// compatibility re-export of the legacy free function; prefer IDatabaseProvider::introspect
pub use introspection::introspect_mysql;
pub use provider::{MySqlOptions, MySqlProvider};
pub use sql_generator::MySqlSqlGenerator;
pub use tls::MySqlTlsMode;
pub use type_mapping::MySqlTypeMapping;

/// Re-exported for callers written against the pre-EF-5 per-provider
/// introspection types; the canonical definitions now live in
/// [`rust_ef::schema`].
pub use rust_ef::schema::{DbColumn, DbTable};
