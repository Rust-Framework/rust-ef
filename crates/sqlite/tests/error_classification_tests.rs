//! Provider error classification (upstream feedback §22).
//!
//! A failed `prepare`/step is classified from SQLite's structured result codes
//! instead of collapsing every driver failure into one category. The
//! undifferentiated `SQLITE_ERROR` bucket keeps the historical `Query`
//! category (see `sync_ops` rustdoc); transport/type classes are covered by the
//! unit tests in `crates/sqlite/src/sync_ops.rs`, since forcing a real
//! busy/locked database would wait for the provider's 5s busy timeout.

use rust_ef::error::EFError;
use rust_ef::provider::IDatabaseProvider;
use rust_ef_sqlite::SqliteProvider;

async fn in_memory() -> SqliteProvider {
    SqliteProvider::new_in_memory().expect("in-memory provider")
}

#[tokio::test]
async fn undifferentiated_sqlite_error_keeps_the_query_category() {
    let provider = in_memory().await;
    let mut conn = provider.get_connection().await.expect("connection");

    let err = conn
        .query("SELECT * FROM missing_table", &[])
        .await
        .expect_err("a missing table must fail at prepare time");

    match err {
        EFError::Query(message, _) => {
            // `SQLITE_ERROR` (code 1) is undifferentiated by design; the driver
            // code and message still reach the operator log.
            assert!(message.contains("SQLite error 1"), "{message}");
            assert!(message.contains("no such table"), "{message}");
        }
        other => panic!("expected EFError::Query for SQLITE_ERROR, got {other:?}"),
    }
}

#[tokio::test]
async fn syntax_error_is_a_statement_shape_failure() {
    let provider = in_memory().await;
    let mut conn = provider.get_connection().await.expect("connection");

    // SQLite reports a token-level syntax error as `SqlInputError` (with the
    // input SQL and byte offset), i.e. a statement-construction failure.
    let err = conn
        .query("SELEC 1", &[])
        .await
        .expect_err("invalid SQL must fail at prepare time");

    assert!(
        matches!(err, EFError::Query(..)),
        "expected EFError::Query for a syntax error, got {err:?}"
    );
}
