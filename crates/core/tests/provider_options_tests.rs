//! Provider options smoke tests.
//!
//! These build providers in lazy mode (no database connection) to verify that
//! the options APIs wire pool/TLS settings correctly for every provider.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_mysql::DbContextOptionsBuilderExt as _;
use rust_ef_postgres::DbContextOptionsBuilderExt as _;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::test]
async fn mysql_options_build_a_provider_without_connecting() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_mysql_with("mysql://user:pass@localhost/rust_ef_test", |options| {
        options.max_pool_size(3).insecure();
    });
    let ctx = DbContext::from_options(&builder.build()).expect("MySQL provider");
    assert_eq!(ctx.provider().name(), "MySQL");
}

#[tokio::test]
async fn postgres_options_build_a_provider_without_connecting() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_postgres_with("host=localhost user=test dbname=rust_ef_test", |options| {
        options.pool_size(3).insecure();
    });
    let ctx = DbContext::from_options(&builder.build()).expect("PostgreSQL provider");
    assert_eq!(ctx.provider().name(), "PostgreSQL");
}

#[tokio::test]
async fn sqlite_options_build_a_file_provider_without_connecting() {
    let path = std::env::temp_dir().join(format!(
        "rust_ef_options_{}_{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_with(path.to_str().expect("utf-8 temp path"), |options| {
        options.max_pool_size(1);
    });
    let ctx = DbContext::from_options(&builder.build()).expect("SQLite provider");
    assert_eq!(ctx.provider().name(), "SQLite");
    let _ = std::fs::remove_file(&path);
}
