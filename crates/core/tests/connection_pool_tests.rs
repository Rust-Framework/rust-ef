//! Phase 3 performance tests — connection pool configuration.
//!
//! Verifies SQLite connection pragmas (WAL journal mode, busy timeout) that
//! the `SqliteProvider` applies on connection open, and that the Postgres /
//! MySQL provider pool-size APIs are present. The PG/MySQL checks are
//! compile-time only (no live server is required).

use rust_ef::provider::IDatabaseProvider;
use rust_ef_sqlite::SqliteProvider;

// ---------------------------------------------------------------------------
// SQLite pragma tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_sqlite_wal_mode_enabled() {
    // In-memory databases always report "memory" for journal_mode, so use a
    // temp file to verify WAL is actually applied by `SqliteProvider::new`.
    let path = std::env::temp_dir().join(format!("rust_ef_wal_test_{}.db", std::process::id()));
    // Clean up any leftovers from a previous run.
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let mode = {
        let provider = SqliteProvider::new(&path).expect("create sqlite provider");
        let mut conn = provider.get_connection().await.expect("get connection");
        let rows = conn
            .query("PRAGMA journal_mode", &[])
            .await
            .expect("pragma");
        assert!(!rows.is_empty(), "PRAGMA journal_mode should return a row");
        String::try_from(rows[0][0].clone()).unwrap().to_lowercase()
    }; // conn + provider dropped here so the file is released.

    assert_eq!(
        mode, "wal",
        "SQLite journal_mode should be wal (set by SqliteProvider::new), got: {}",
        mode
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[tokio::test]
async fn test_sqlite_busy_timeout_set() {
    let path = std::env::temp_dir().join(format!("rust_ef_busy_test_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));

    let timeout = {
        let provider = SqliteProvider::new(&path).expect("create sqlite provider");
        let mut conn = provider.get_connection().await.expect("get connection");
        let rows = conn
            .query("PRAGMA busy_timeout", &[])
            .await
            .expect("pragma");
        assert!(!rows.is_empty(), "PRAGMA busy_timeout should return a row");
        i64::try_from(rows[0][0].clone()).expect("busy_timeout should be numeric")
    };

    assert!(
        timeout >= 5000,
        "busy_timeout should be >= 5000ms (set by SqliteProvider::new), got: {}",
        timeout
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

// ---------------------------------------------------------------------------
// Postgres / MySQL pool-size API — compile-time checks (no server required)
// ---------------------------------------------------------------------------

#[test]
fn test_postgres_pool_size_api_exists() {
    use rust_ef_postgres::DbContextOptionsBuilderExt;
    // Compile-time check: `PostgresProvider::new` accepts a pool-size
    // argument, and `use_postgres_with_pool` resolves on the options builder.
    // We reference the items without invoking them so no PG server is needed.
    let _new = rust_ef_postgres::PostgresProvider::new;
    let _with_pool =
        <rust_ef::db_context::DbContextOptionsBuilder as DbContextOptionsBuilderExt>::use_postgres_with_pool;
}

#[test]
fn test_mysql_pool_options_api_exists() {
    use rust_ef_mysql::DbContextOptionsBuilderExt;
    // Compile-time check: `use_mysql_with_pool` resolves on the options
    // builder. We reference the method without calling it — sqlx's
    // `MySqlPoolOptions` is a transitive dep not re-exported here, and no
    // MySQL server is required.
    let _with_pool =
        <rust_ef::db_context::DbContextOptionsBuilder as DbContextOptionsBuilderExt>::use_mysql_with_pool;
}
