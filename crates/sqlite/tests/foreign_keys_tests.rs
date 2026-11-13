//! EF-4: SQLite foreign-key enforcement is a connection-level switch.
//!
//! `SqliteOptions::foreign_keys(true)` applies `PRAGMA foreign_keys=ON` on the
//! same acquisition hook as `journal_mode=WAL`/`busy_timeout`, so every
//! borrowed pooled connection (and the shared in-memory connection) reports
//! `PRAGMA foreign_keys = 1` and enforces `REFERENCES` constraints.

use rust_ef::db_context::DbContextOptionsBuilder;
use rust_ef::error::EFError;
use rust_ef::provider::{DbValue, IDatabaseProvider};
use rust_ef_sqlite::DbContextOptionsBuilderExt;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_db_path() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "rust_ef_fk_{}_{}_{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    path
}

async fn pragma_i64(provider: &dyn IDatabaseProvider, pragma: &str) -> i64 {
    let mut conn = provider.get_connection().await.expect("connection");
    let rows = conn.query(pragma, &[]).await.expect("pragma query");
    match &rows[0][0] {
        DbValue::I64(value) => *value,
        other => panic!("expected integer from {pragma}, got {other:?}"),
    }
}

async fn create_parent_child(provider: &dyn IDatabaseProvider) {
    provider
        .execute_migration_command(
            "CREATE TABLE parent (id INTEGER PRIMARY KEY); \
             CREATE TABLE child (\
                 id INTEGER PRIMARY KEY, \
                 parent_id INTEGER NOT NULL REFERENCES parent(id)\
             );",
        )
        .await
        .expect("create tables");
}

#[tokio::test]
async fn every_pooled_connection_enforces_foreign_keys() {
    let path = temp_db_path();
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_with(&path.to_string_lossy(), |options| {
        options.max_pool_size(3).foreign_keys(true);
    });
    let options = builder.build();
    let provider = options.create_provider().expect("sqlite provider");

    create_parent_child(&*provider).await;

    // Hold several connections at once so the pool has to grow; the PRAGMAs
    // must be present on every one of them, not just the first.
    let mut borrowed = Vec::new();
    for _ in 0..3 {
        let mut conn = provider.get_connection().await.expect("connection");
        let rows = conn
            .query(
                "PRAGMA foreign_keys; PRAGMA busy_timeout; PRAGMA journal_mode",
                &[],
            )
            .await;
        // SQLite only reports the first statement's rows for a multi-PRAGMA
        // string through rusqlite, so assert each one separately.
        drop(rows);
        let fk = conn
            .query("PRAGMA foreign_keys", &[])
            .await
            .expect("foreign_keys pragma");
        match &fk[0][0] {
            DbValue::I64(value) => assert_eq!(*value, 1, "foreign_keys must be ON"),
            other => panic!("expected integer, got {other:?}"),
        }
        let busy = conn
            .query("PRAGMA busy_timeout", &[])
            .await
            .expect("busy_timeout pragma");
        match &busy[0][0] {
            DbValue::I64(value) => assert_eq!(*value, 5000),
            other => panic!("expected integer, got {other:?}"),
        }
        borrowed.push(conn);
    }

    // Enforcement, not just the flag: an orphan child row must fail.
    let mut conn = borrowed.pop().expect("connection");
    let err = conn
        .execute("INSERT INTO child (id, parent_id) VALUES (1, 999)", &[])
        .await
        .expect_err("orphan FK insert must be rejected");
    assert!(
        matches!(err, EFError::Query(..)),
        "expected a constraint error, got {err:?}"
    );

    drop(borrowed);
    drop(conn);
    drop(provider);
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn in_memory_foreign_keys_option_applies() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory_with(|options| {
        options.foreign_keys(true);
    });
    let provider = builder
        .build()
        .create_provider()
        .expect("in-memory provider");

    create_parent_child(&*provider).await;

    assert_eq!(
        pragma_i64(&*provider, "PRAGMA foreign_keys").await,
        1,
        "in-memory connection must enforce foreign keys"
    );

    let mut conn = provider.get_connection().await.expect("connection");
    let err = conn
        .execute("INSERT INTO child (id, parent_id) VALUES (1, 999)", &[])
        .await
        .expect_err("orphan FK insert must be rejected");
    assert!(
        matches!(err, EFError::Query(..)),
        "expected a constraint error, got {err:?}"
    );
}
