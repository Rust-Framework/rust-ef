//! EF-2: SQLite unique / primary-key violations surface as
//! [`EFError::UniqueViolation`] instead of an untyped `EFError::Query`.

use rust_ef::error::EFError;
use rust_ef::provider::IDatabaseProvider;
use rust_ef_sqlite::SqliteProvider;

async fn provider_with_unique_table() -> SqliteProvider {
    let provider = SqliteProvider::new_in_memory().expect("in-memory provider");
    provider
        .execute_migration_command(
            "CREATE TABLE order_record (\
                 id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 tenant_id INTEGER NOT NULL, \
                 request_key TEXT NOT NULL, \
                 UNIQUE (tenant_id, request_key)\
             );",
        )
        .await
        .expect("create table");
    provider
}

#[tokio::test]
async fn sqlite_unique_constraint_violation_is_typed() {
    let provider = provider_with_unique_table().await;
    let mut conn = provider.get_connection().await.expect("connection");

    conn.execute(
        "INSERT INTO order_record (tenant_id, request_key) VALUES (1, 'k')",
        &[],
    )
    .await
    .expect("first insert");

    let err = conn
        .execute(
            "INSERT INTO order_record (tenant_id, request_key) VALUES (1, 'k')",
            &[],
        )
        .await
        .expect_err("duplicate must fail");

    match err {
        EFError::UniqueViolation { table, constraint } => {
            assert_eq!(table.as_deref(), Some("order_record"));
            // SQLite reports columns, not a constraint name.
            assert_eq!(constraint, None);
        }
        other => panic!("expected EFError::UniqueViolation, got {other:?}"),
    }
}

#[tokio::test]
async fn sqlite_primary_key_violation_is_typed() {
    let provider = provider_with_unique_table().await;
    let mut conn = provider.get_connection().await.expect("connection");

    conn.execute(
        "INSERT INTO order_record (id, tenant_id, request_key) VALUES (7, 1, 'a')",
        &[],
    )
    .await
    .expect("first insert");

    let err = conn
        .execute(
            "INSERT INTO order_record (id, tenant_id, request_key) VALUES (7, 2, 'b')",
            &[],
        )
        .await
        .expect_err("duplicate primary key must fail");

    assert!(
        matches!(
            err,
            EFError::UniqueViolation {
                table: Some(ref table),
                ..
            } if table == "order_record"
        ),
        "expected primary-key UniqueViolation, got {err:?}"
    );
}

#[tokio::test]
async fn sqlite_non_unique_errors_keep_their_typed_category() {
    let provider = provider_with_unique_table().await;
    let mut conn = provider.get_connection().await.expect("connection");

    let err = conn
        .execute(
            "INSERT INTO order_record (tenant_id, request_key) VALUES (NULL, 'k')",
            &[],
        )
        .await
        .expect_err("NOT NULL violation must fail");

    assert!(
        matches!(err, EFError::Query(..)),
        "expected EFError::Query for NOT NULL violation, got {err:?}"
    );
}
