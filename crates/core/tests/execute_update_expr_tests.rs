//! Expression updates (`SET col = <expr>`) for `ExecuteUpdateBuilder`
//! (rust-ef 2.7 / EF-03).
//!
//! Covers the SQL shape and parameter order of `set_column_expr`,
//! `increment`/`decrement`, and SQLite execution of atomic increments,
//! decrements, column-to-column copies and compare-and-swap version bumps.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::query::ColumnExpr;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("eu_counters")]
struct EuCounter {
    #[primary_key]
    id: i32,
    version: i32,
    views: i64,
    stock: i64,
    reserved: i64,
    status: String,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Seeds two counters at version 1.
async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<EuCounter>();
    ctx.ensure_created().await.unwrap();
    for (id, views, stock, reserved) in [(1, 10_i64, 100_i64, 5_i64), (2, 20, 50, 7)] {
        ctx.add::<EuCounter>(EuCounter {
            id,
            version: 1,
            views,
            stock,
            reserved,
            status: "pending".into(),
        });
    }
    ctx.save_changes().await.unwrap();
    ctx
}

async fn read(ctx: &mut DbContext, id: i32) -> EuCounter {
    ctx.set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == id))
        .first_or_default()
        .await
        .unwrap()
        .expect("counter exists")
}

// ---------------------------------------------------------------------------
// SQL shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn set_column_expr_builds_arithmetic_set() {
    let mut ctx = make_ctx();
    ctx.set::<EuCounter>();

    let sql = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 1))
        .execute_update()
        .set_column_expr(
            EuCounter::COLUMN_VIEWS,
            ColumnExpr::column(EuCounter::COLUMN_VIEWS).add(ColumnExpr::value(10_i64)),
        )
        .to_sql();

    assert_eq!(
        sql, "UPDATE eu_counters SET views = views + ? WHERE id = ?",
        "SET expression SQL shape"
    );
}

#[tokio::test]
async fn increment_and_decrement_build_arithmetic_set() {
    let mut ctx = make_ctx();
    ctx.set::<EuCounter>();

    let update = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.version == 1))
        .execute_update()
        .set_column(EuCounter::COLUMN_STATUS, "paid")
        .increment(EuCounter::COLUMN_VERSION, 1)
        .decrement(EuCounter::COLUMN_STOCK, 5_i64);

    assert_eq!(
        update.to_sql(),
        "UPDATE eu_counters SET status = ?, version = version + ?, stock = stock - ? \
         WHERE version = ?"
    );
    // SET parameters first (assignment order), then WHERE parameters.
    assert_eq!(
        update.params(),
        vec![
            DbValue::from("paid"),
            DbValue::from(1_i32),
            DbValue::from(5_i64),
            DbValue::from(1_i32),
        ]
    );
}

#[tokio::test]
async fn expression_params_precede_where_params() {
    let mut ctx = make_ctx();
    ctx.set::<EuCounter>();

    let update = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 1 && c.version == 1))
        .execute_update()
        // value-first operand still binds in SQL order: `? + views`
        .set_column_expr(
            EuCounter::COLUMN_VIEWS,
            ColumnExpr::value(7_i64).add(ColumnExpr::column(EuCounter::COLUMN_VIEWS)),
        )
        .set_column_expr(
            EuCounter::COLUMN_STOCK,
            ColumnExpr::column(EuCounter::COLUMN_STOCK).sub(ColumnExpr::value(3_i64)),
        );

    let sql = update.to_sql();
    assert_eq!(
        sql,
        "UPDATE eu_counters SET views = ? + views, stock = stock - ? \
         WHERE (id = ?) AND (version = ?)"
    );
    assert_eq!(sql.matches('?').count(), 4);
    assert_eq!(
        update.params(),
        vec![
            DbValue::from(7_i64),
            DbValue::from(3_i64),
            DbValue::from(1_i32),
            DbValue::from(1_i32),
        ],
        "SET expression values must come before WHERE values"
    );
}

#[tokio::test]
async fn nested_subtraction_is_parenthesized_and_params_stay_ordered() {
    let mut ctx = make_ctx();
    ctx.set::<EuCounter>();

    let update = ctx
        .set::<EuCounter>()
        .query()
        .execute_update()
        .set_column_expr(
            EuCounter::COLUMN_STOCK,
            ColumnExpr::column(EuCounter::COLUMN_STOCK)
                .sub(ColumnExpr::column(EuCounter::COLUMN_RESERVED).add(ColumnExpr::value(1_i64))),
        );

    assert_eq!(
        update.to_sql(),
        "UPDATE eu_counters SET stock = stock - (reserved + ?)"
    );
    assert_eq!(update.params(), vec![DbValue::from(1_i64)]);
}

#[tokio::test]
async fn set_column_still_binds_a_single_value_per_assignment() {
    let mut ctx = make_ctx();
    ctx.set::<EuCounter>();

    let update = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 2))
        .execute_update()
        .set_column(EuCounter::COLUMN_STATUS, "closed")
        .set_column(EuCounter::COLUMN_VIEWS, 99_i64);

    assert_eq!(
        update.to_sql(),
        "UPDATE eu_counters SET status = ?, views = ? WHERE id = ?"
    );
    assert_eq!(
        update.params(),
        vec![
            DbValue::from("closed"),
            DbValue::from(99_i64),
            DbValue::from(2_i32),
        ]
    );
}

// ---------------------------------------------------------------------------
// SQLite execution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn increment_and_decrement_run_against_sqlite() {
    let mut ctx = seed().await;

    let affected = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 1))
        .execute_update()
        .set_column(EuCounter::COLUMN_STATUS, "paid")
        .increment(EuCounter::COLUMN_VERSION, 1)
        .decrement(EuCounter::COLUMN_STOCK, 5_i64)
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 1);

    let counter = read(&mut ctx, 1).await;
    assert_eq!(counter.status, "paid");
    assert_eq!(counter.version, 2, "version bump must persist");
    assert_eq!(counter.stock, 95, "stock decrement must persist");
    assert_eq!(counter.views, 10, "untouched columns stay unchanged");
}

#[tokio::test]
async fn column_to_column_expression_runs_against_sqlite() {
    let mut ctx = seed().await;

    // stock = stock + reserved
    let affected = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 1))
        .execute_update()
        .set_column_expr(
            EuCounter::COLUMN_STOCK,
            ColumnExpr::column(EuCounter::COLUMN_STOCK)
                .add(ColumnExpr::column(EuCounter::COLUMN_RESERVED)),
        )
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 1);

    let counter = read(&mut ctx, 1).await;
    assert_eq!(counter.stock, 105, "stock = stock + reserved");
    assert_eq!(counter.reserved, 5, "the source column is not modified");
}

#[tokio::test]
async fn column_to_column_copy_runs_against_sqlite() {
    let mut ctx = seed().await;

    // views = reserved
    let affected = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 2))
        .execute_update()
        .set_column_expr(
            EuCounter::COLUMN_VIEWS,
            ColumnExpr::column(EuCounter::COLUMN_RESERVED),
        )
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 1);

    let counter = read(&mut ctx, 2).await;
    assert_eq!(counter.views, 7, "views must be copied from reserved");
}

// ---------------------------------------------------------------------------
// CAS: WHERE version guard + SET version = version + 1 in one statement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cas_guard_with_increment_affected_rows_semantics() {
    let mut ctx = seed().await;

    // Winner: guard matches, version bumped atomically in the same statement.
    let affected = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 1 && c.version == 1))
        .execute_update()
        .set_column(EuCounter::COLUMN_STATUS, "paid")
        .increment(EuCounter::COLUMN_VERSION, 1)
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 1);

    // Loser: the stale version guard no longer matches → 0 rows; the SET must
    // not be applied even though it would be valid on its own.
    let affected = ctx
        .set::<EuCounter>()
        .query()
        .filter(linq!(|c: EuCounter| c.id == 1 && c.version == 1))
        .execute_update()
        .set_column(EuCounter::COLUMN_STATUS, "refunded")
        .increment(EuCounter::COLUMN_VERSION, 1)
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 0, "stale CAS guard must not update the row");

    let counter = read(&mut ctx, 1).await;
    assert_eq!(counter.status, "paid", "the losing SET was not applied");
    assert_eq!(counter.version, 2, "version was bumped exactly once");
}
