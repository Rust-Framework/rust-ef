//! §11: atomic upsert-increment on the existing `ctx.upsert()` path.
//!
//! `DbContext::upsert_increment` keeps the single UPSERT write path and switches
//! the conflict branch for the listed columns from "overwrite" to "add":
//! `SET col = col + EXCLUDED.col` (MySQL: `SET col = col + VALUES(col)`). The
//! inserted value is the delta and is therefore bound exactly once.
//!
//! Covered here:
//! - SQL shape for the three dialects (SQLite/PostgreSQL `ON CONFLICT`,
//!   MySQL `ON DUPLICATE KEY UPDATE`);
//! - SQLite end to end: (1) miss → INSERT, (2) key conflict → increment,
//!   (3) plain `upsert` still overwrites (the switch is per call).

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef::provider::ISqlGenerator;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

/// Composite-key counter row: the primary key identifies the counter, so the
/// upsert conflict target is the key itself (no surrogate id needed).
#[derive(Debug, Clone, EntityType)]
#[table("usage_counters")]
struct UsageCounter {
    #[primary_key]
    tenant_id: i32,
    #[primary_key]
    #[max_length(32)]
    metric: String,
    quantity: i64,
    #[required]
    #[max_length(64)]
    note: String,
}

const COLUMNS: &[&str] = &["tenant_id", "metric", "quantity", "note"];
const CONFLICT_COLUMNS: &[&str] = &["tenant_id", "metric"];
const INCREMENT_COLUMNS: &[&str] = &["quantity"];

// ---------------------------------------------------------------------------
// SQL shape per dialect
// ---------------------------------------------------------------------------

#[test]
fn sqlite_increments_the_conflict_value() {
    let sql = rust_ef_sqlite::SqliteSqlGenerator::new().upsert_batch_increment(
        "usage_counters",
        COLUMNS,
        CONFLICT_COLUMNS,
        INCREMENT_COLUMNS,
        1,
    );

    assert_eq!(
        sql,
        "INSERT INTO \"usage_counters\" (\"tenant_id\", \"metric\", \"quantity\", \"note\") \
         VALUES (?, ?, ?, ?) ON CONFLICT(\"tenant_id\", \"metric\") DO UPDATE SET \
         \"quantity\" = \"quantity\" + EXCLUDED.\"quantity\", \"note\" = EXCLUDED.\"note\""
    );
}

#[test]
fn postgres_numbers_placeholders_and_increments_the_conflict_value() {
    let sql = rust_ef_postgres::PostgresSqlGenerator::new().upsert_batch_increment(
        "usage_counters",
        COLUMNS,
        CONFLICT_COLUMNS,
        INCREMENT_COLUMNS,
        2,
    );

    assert_eq!(
        sql,
        "INSERT INTO \"usage_counters\" (\"tenant_id\", \"metric\", \"quantity\", \"note\") \
         VALUES ($1, $2, $3, $4), ($5, $6, $7, $8) ON CONFLICT(\"tenant_id\", \"metric\") \
         DO UPDATE SET \"quantity\" = \"quantity\" + EXCLUDED.\"quantity\", \
         \"note\" = EXCLUDED.\"note\""
    );
}

#[test]
fn mysql_uses_on_duplicate_key_update_with_values() {
    let sql = rust_ef_mysql::MySqlSqlGenerator::new().upsert_batch_increment(
        "usage_counters",
        COLUMNS,
        CONFLICT_COLUMNS,
        INCREMENT_COLUMNS,
        1,
    );

    assert_eq!(
        sql,
        "INSERT INTO `usage_counters` (`tenant_id`, `metric`, `quantity`, `note`) \
         VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE \
         `quantity` = `quantity` + VALUES(`quantity`), `note` = VALUES(`note`)"
    );
}

// ---------------------------------------------------------------------------
// SQLite end to end
// ---------------------------------------------------------------------------

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.set::<UsageCounter>();
    ctx
}

async fn rows(ctx: &mut DbContext) -> Vec<UsageCounter> {
    ctx.set::<UsageCounter>()
        .query()
        .to_list()
        .await
        .expect("query")
}

fn counter(quantity: i64, note: &str) -> UsageCounter {
    UsageCounter {
        tenant_id: 1,
        metric: "requests".into(),
        quantity,
        note: note.into(),
    }
}

#[tokio::test]
async fn upsert_increment_inserts_then_adds_on_key_conflict() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    // (1) No row yet → INSERT; the delta becomes the initial value.
    ctx.upsert_increment::<UsageCounter>(counter(5, "first"), &[UsageCounter::COLUMN_QUANTITY]);
    ctx.save_changes().await.expect("first upsert");

    let loaded = rows(&mut ctx).await;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].quantity, 5, "first delta is inserted");
    assert_eq!(loaded[0].note, "first");

    // (2) Same key → conflict → quantity += 7, still one row. The inserted
    // value (7) is bound once and both inserted and added.
    ctx.set::<UsageCounter>().clear_entries();
    ctx.upsert_increment::<UsageCounter>(counter(7, "second"), &[UsageCounter::COLUMN_QUANTITY]);
    ctx.save_changes().await.expect("second upsert");

    let loaded = rows(&mut ctx).await;
    assert_eq!(loaded.len(), 1, "conflict must not create a duplicate row");
    assert_eq!(loaded[0].quantity, 12, "5 + 7");
    assert_eq!(
        loaded[0].note, "second",
        "non-key columns keep the plain upsert overwrite behaviour"
    );

    // (3) Different key → an independent counter is inserted with its own delta.
    ctx.set::<UsageCounter>().clear_entries();
    ctx.upsert_increment::<UsageCounter>(
        UsageCounter {
            tenant_id: 1,
            metric: "storage".into(),
            quantity: 2,
            note: "storage".into(),
        },
        &[UsageCounter::COLUMN_QUANTITY],
    );
    ctx.save_changes().await.expect("third upsert");

    let loaded = rows(&mut ctx).await;
    assert_eq!(loaded.len(), 2);
    let storage = loaded
        .iter()
        .find(|row| row.metric == "storage")
        .expect("storage counter");
    assert_eq!(storage.quantity, 2);
}

#[tokio::test]
async fn plain_upsert_still_overwrites_the_counter() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    ctx.upsert_increment::<UsageCounter>(counter(5, "first"), &[UsageCounter::COLUMN_QUANTITY]);
    ctx.save_changes().await.expect("seed");

    // A plain upsert on the same key keeps the old overwrite semantics; only
    // `upsert_increment` switches the listed columns to `col = col + excluded`.
    ctx.set::<UsageCounter>().clear_entries();
    ctx.upsert::<UsageCounter>(counter(99, "plain"));
    ctx.save_changes().await.expect("plain upsert");

    let loaded = rows(&mut ctx).await;
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].quantity, 99, "plain upsert overwrites");
}

#[tokio::test]
async fn incrementing_a_key_column_is_rejected() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    // tenant_id is part of the conflict target; `col = col + excluded.col` on a
    // key column is not a valid increment, so save_changes must reject it
    // instead of sending broken SQL.
    ctx.upsert_increment::<UsageCounter>(counter(5, "bad"), &[UsageCounter::COLUMN_TENANT_ID]);
    let error = ctx.save_changes().await.unwrap_err();
    assert!(
        matches!(error, rust_ef::error::EFError::Configuration(..)),
        "expected a configuration error, got {error:?}"
    );
}
