//! PostgreSQL provider for Rust Entity Framework.

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

pub use connection::PostgresConnection;
pub use di_extension::DbContextOptionsBuilderExt;
#[allow(deprecated)]
// compatibility re-export of the legacy free function; prefer IDatabaseProvider::introspect
pub use introspection::introspect_postgres;
pub use provider::{PostgresOptions, PostgresProvider};
pub use sql_generator::PostgresSqlGenerator;
pub use tls::PgTlsMode;
pub use type_mapping::PostgresTypeMapping;

/// Re-exported for callers written against the pre-EF-5 per-provider
/// introspection types; the canonical definitions now live in
/// [`rust_ef::schema`].
pub use rust_ef::schema::{DbColumn, DbTable};
