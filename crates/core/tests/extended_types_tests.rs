//! Integration tests for chrono / uuid / decimal type support.
//!
//! Requires all three features: `cargo test -p rust-ef --features chrono,uuid,decimal -- --skip postgres --skip mysql`

#![cfg(all(feature = "chrono", feature = "uuid", feature = "decimal"))]

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

// ---------------------------------------------------------------------------
// Entity with chrono / uuid / decimal fields
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, EntityType)]
#[table("transactions")]
struct Transaction {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(100)]
    pub name: String,

    /// UTC timestamp — stored as RFC 3339 string, DDL: TIMESTAMPTZ / DATETIME / TEXT
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Naive datetime — stored as "YYYY-MM-DD HH:MM:SS", DDL: TIMESTAMP / DATETIME / TEXT
    pub processed_at: chrono::NaiveDateTime,

    /// Date only — DDL: DATE / DATE / TEXT
    pub transaction_date: chrono::NaiveDate,

    /// UUID — stored as hyphenated string, DDL: UUID / CHAR(36) / TEXT
    pub reference_id: uuid::Uuid,

    /// Decimal — stored as string, DDL: NUMERIC / DECIMAL(38,18) / TEXT
    pub amount: rust_decimal::Decimal,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.discover_entities().expect("discover");
    ctx.set::<Transaction>();
    ctx
}

fn sample_transaction() -> Transaction {
    Transaction {
        id: 0,
        name: "Payment".into(),
        created_at: chrono::DateTime::parse_from_rfc3339("2026-06-26T12:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        processed_at: chrono::NaiveDateTime::parse_from_str(
            "2026-06-26 14:00:00",
            "%Y-%m-%d %H:%M:%S",
        )
        .unwrap(),
        transaction_date: chrono::NaiveDate::from_ymd_opt(2026, 6, 26).unwrap(),
        reference_id: uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        amount: rust_decimal::Decimal::new(1234567, 2), // 12345.67
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn chrono_uuid_decimal_round_trip() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    let original = sample_transaction();
    ctx.set::<Transaction>().add(original.clone());
    ctx.save_changes().await.expect("save");

    let rows = ctx
        .set::<Transaction>()
        .query()
        .to_list()
        .await
        .expect("query");
    assert_eq!(rows.len(), 1, "should have 1 transaction");

    let row = &rows[0];
    assert_eq!(row.name, "Payment");
    assert_eq!(row.created_at, original.created_at, "DateTime round-trip");
    assert_eq!(
        row.processed_at, original.processed_at,
        "NaiveDateTime round-trip"
    );
    assert_eq!(
        row.transaction_date, original.transaction_date,
        "NaiveDate round-trip"
    );
    assert_eq!(row.reference_id, original.reference_id, "Uuid round-trip");
    assert_eq!(row.amount, original.amount, "Decimal round-trip");
}

#[tokio::test]
async fn multiple_transactions_query() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    for i in 0..3 {
        let mut tx = sample_transaction();
        tx.name = format!("Payment-{}", i);
        tx.amount = rust_decimal::Decimal::new((i + 1) as i64 * 1000, 2);
        tx.reference_id = uuid::Uuid::new_v4();
        ctx.set::<Transaction>().add(tx);
    }
    ctx.save_changes().await.expect("save");

    let rows = ctx
        .set::<Transaction>()
        .query()
        .to_list()
        .await
        .expect("query");
    assert_eq!(rows.len(), 3, "should have 3 transactions");

    // Verify each has a distinct UUID
    let ids: std::collections::HashSet<_> = rows.iter().map(|r| r.reference_id).collect();
    assert_eq!(ids.len(), 3, "UUIDs should be distinct");

    // Verify amounts
    let amounts: Vec<_> = rows.iter().map(|r| r.amount).collect();
    assert!(amounts.contains(&rust_decimal::Decimal::new(1000, 2)));
    assert!(amounts.contains(&rust_decimal::Decimal::new(2000, 2)));
    assert!(amounts.contains(&rust_decimal::Decimal::new(3000, 2)));
}

#[tokio::test]
async fn datetime_filter_query() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    let early = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let late = chrono::DateTime::parse_from_rfc3339("2026-12-31T23:59:59Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    ctx.set::<Transaction>().add(Transaction {
        id: 0,
        name: "Early".into(),
        created_at: early,
        processed_at: chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
        transaction_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        reference_id: uuid::Uuid::new_v4(),
        amount: rust_decimal::Decimal::new(100, 2),
    });
    ctx.set::<Transaction>().add(Transaction {
        id: 0,
        name: "Late".into(),
        created_at: late,
        processed_at: chrono::NaiveDate::from_ymd_opt(2026, 12, 31)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap(),
        transaction_date: chrono::NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
        reference_id: uuid::Uuid::new_v4(),
        amount: rust_decimal::Decimal::new(200, 2),
    });
    ctx.save_changes().await.expect("save");

    let all = ctx
        .set::<Transaction>()
        .query()
        .to_list()
        .await
        .expect("query");
    assert_eq!(all.len(), 2);
    assert!(all.iter().any(|r| r.name == "Early"));
    assert!(all.iter().any(|r| r.name == "Late"));
    assert_ne!(all[0].created_at, all[1].created_at);
}

#[tokio::test]
async fn update_with_chrono_fields() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    ctx.set::<Transaction>().add(sample_transaction());
    ctx.save_changes().await.expect("save");

    // Load, modify, save
    ctx.set::<Transaction>().load_all().await.expect("load");
    let new_time = chrono::DateTime::parse_from_rfc3339("2027-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    for tx in ctx.set::<Transaction>().tracked_entries_mut() {
        tx.created_at = new_time;
        tx.amount = rust_decimal::Decimal::new(99999, 2);
    }
    ctx.set::<Transaction>().detect_changes();
    let result = ctx.save_changes().await.expect("update save");
    assert_eq!(result.updated, 1, "1 row should be updated");

    // Verify
    let rows = ctx
        .set::<Transaction>()
        .query()
        .to_list()
        .await
        .expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].created_at, new_time, "DateTime should be updated");
    assert_eq!(
        rows[0].amount,
        rust_decimal::Decimal::new(99999, 2),
        "Decimal should be updated"
    );
}

#[test]
fn map_column_type_chrono_uuid_decimal() {
    use rust_ef::migration::{MigrationDialect, SnapshotColumn};

    let make_col = |type_name: &str| SnapshotColumn {
        field_name: "test".into(),
        column_name: "test".into(),
        type_name: type_name.into(),
        is_primary_key: false,
        is_required: true,
        is_foreign_key: false,
        max_length: None,
        is_auto_increment: false,
        is_sequence: false,
        sequence_name: None,
        fk_referenced_table: None,
        fk_referenced_column: None,
        has_index: false,
        is_unique: false,
        fk_on_delete: None,
    };

    // chrono::DateTime<Utc> — std::any::type_name produces "chrono::DateTime<chrono::Utc>"
    let dt_col = make_col("chrono::DateTime<chrono::Utc>");
    assert_eq!(
        MigrationDialect::Postgres.map_column_type(&dt_col),
        "TIMESTAMPTZ"
    );
    assert_eq!(MigrationDialect::MySql.map_column_type(&dt_col), "DATETIME");
    assert_eq!(MigrationDialect::Sqlite.map_column_type(&dt_col), "TEXT");

    // NaiveDateTime
    let ndt_col = make_col("chrono::NaiveDateTime");
    assert_eq!(
        MigrationDialect::Postgres.map_column_type(&ndt_col),
        "TIMESTAMP"
    );
    assert_eq!(
        MigrationDialect::MySql.map_column_type(&ndt_col),
        "DATETIME"
    );
    assert_eq!(MigrationDialect::Sqlite.map_column_type(&ndt_col), "TEXT");

    // NaiveDate
    let nd_col = make_col("chrono::NaiveDate");
    assert_eq!(MigrationDialect::Postgres.map_column_type(&nd_col), "DATE");
    assert_eq!(MigrationDialect::MySql.map_column_type(&nd_col), "DATE");
    assert_eq!(MigrationDialect::Sqlite.map_column_type(&nd_col), "TEXT");

    // Uuid
    let uuid_col = make_col("uuid::Uuid");
    assert_eq!(
        MigrationDialect::Postgres.map_column_type(&uuid_col),
        "UUID"
    );
    assert_eq!(
        MigrationDialect::MySql.map_column_type(&uuid_col),
        "CHAR(36)"
    );
    assert_eq!(MigrationDialect::Sqlite.map_column_type(&uuid_col), "TEXT");

    // Decimal
    let dec_col = make_col("rust_decimal::Decimal");
    assert_eq!(
        MigrationDialect::Postgres.map_column_type(&dec_col),
        "NUMERIC"
    );
    assert_eq!(
        MigrationDialect::MySql.map_column_type(&dec_col),
        "DECIMAL(38,18)"
    );
    assert_eq!(MigrationDialect::Sqlite.map_column_type(&dec_col), "TEXT");
}

#[test]
fn map_column_type_existing_types_still_match() {
    use rust_ef::migration::{MigrationDialect, SnapshotColumn};

    let make_col = |type_name: &str, max_length: Option<usize>| SnapshotColumn {
        field_name: "test".into(),
        column_name: "test".into(),
        type_name: type_name.into(),
        is_primary_key: false,
        is_required: true,
        is_foreign_key: false,
        max_length,
        is_auto_increment: false,
        is_sequence: false,
        sequence_name: None,
        fk_referenced_table: None,
        fk_referenced_column: None,
        has_index: false,
        is_unique: false,
        fk_on_delete: None,
    };

    // Fully-qualified String (from std::any::type_name)
    let s_col = make_col("alloc::string::String", Some(200));
    assert_eq!(
        MigrationDialect::Sqlite.map_column_type(&s_col),
        "VARCHAR(200)",
        "qualified String should match"
    );

    // Simple String (from hand-written metas)
    let s_col2 = make_col("String", Some(100));
    assert_eq!(
        MigrationDialect::Sqlite.map_column_type(&s_col2),
        "VARCHAR(100)",
        "simple String should match"
    );

    // i32
    let i_col = make_col("i32", None);
    assert_eq!(MigrationDialect::Sqlite.map_column_type(&i_col), "INTEGER");

    // bool
    let b_col = make_col("bool", None);
    assert_eq!(MigrationDialect::Sqlite.map_column_type(&b_col), "BOOLEAN");
}
