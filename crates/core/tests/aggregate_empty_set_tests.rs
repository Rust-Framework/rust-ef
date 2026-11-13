//! EF-17: aggregate terminals over an empty input set.
//!
//! SQL enters an aggregate query with an empty input set by returning a single
//! row whose cell is `NULL`. Before 2.4.0 `sum`/`avg` tried to convert that
//! `NULL` cell to `f64` and returned
//! `EFError::TypeConversion("SUM result is not f64")` instead of the neutral
//! value. `min`/`max` already returned `None` (their signatures are
//! `Option<V>`); `count` is unaffected (it counts rows, never sees `NULL`).

mod common;

use common::TestItem;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build()).expect("context");
    ctx.set::<TestItem>();
    ctx
}

async fn seed_ctx(ctx: &mut DbContext) {
    ctx.ensure_created().await.expect("ensure_created");
    for i in 1..=5 {
        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: format!("item-{i}"),
            value: i as f64,
        });
    }
    ctx.save_changes().await.expect("seed save");
    ctx.set::<TestItem>().clear_entries();
}

#[tokio::test]
async fn empty_sum_and_avg_return_zero() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    let sum: f64 = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; sum b.value)
        .await
        .expect("SUM over an empty set must not error");
    assert_eq!(sum, 0.0, "SUM over an empty set is the neutral value 0.0");

    let avg: f64 = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; avg b.value)
        .await
        .expect("AVG over an empty set must not error");
    assert_eq!(avg, 0.0, "AVG over an empty set is the neutral value 0.0");
}

#[tokio::test]
async fn filtered_to_empty_sum_and_avg_return_zero() {
    let mut ctx = make_ctx();
    seed_ctx(&mut ctx).await;

    let sum: f64 = linq!(ctx.set::<TestItem>().query(), |b: TestItem| b.value > 100.0; sum b.value)
        .await
        .expect("filtered SUM must not error");
    assert_eq!(sum, 0.0);

    let avg: f64 = linq!(ctx.set::<TestItem>().query(), |b: TestItem| b.value > 100.0; avg b.value)
        .await
        .expect("filtered AVG must not error");
    assert_eq!(avg, 0.0);
}

#[tokio::test]
async fn count_is_unaffected_by_the_empty_set_normalization() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    let count = ctx
        .set::<TestItem>()
        .query()
        .count()
        .await
        .expect("COUNT over an empty set");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn empty_min_and_max_stay_none() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    let min: Option<f64> = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; min b.value)
        .await
        .expect("MIN over an empty set must not error");
    assert_eq!(min, None, "MIN of an empty set is NULL → None");

    let max: Option<f64> = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; max b.value)
        .await
        .expect("MAX over an empty set must not error");
    assert_eq!(max, None, "MAX of an empty set is NULL → None");
}

#[tokio::test]
async fn non_empty_results_are_unchanged() {
    let mut ctx = make_ctx();
    seed_ctx(&mut ctx).await;

    let sum: f64 = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; sum b.value)
        .await
        .expect("SUM");
    assert!(
        (sum - 15.0).abs() < f64::EPSILON,
        "expected 15.0, got {sum}"
    );

    let avg: f64 = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; avg b.value)
        .await
        .expect("AVG");
    assert!((avg - 3.0).abs() < f64::EPSILON, "expected 3.0, got {avg}");

    let min: Option<f64> = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; min b.value)
        .await
        .expect("MIN");
    assert_eq!(min, Some(1.0));

    let max: Option<f64> = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; max b.value)
        .await
        .expect("MAX");
    assert_eq!(max, Some(5.0));
}

/// SQLite stores `SUM` over a nullable column with all-`NULL` values as
/// `NULL` too — the neutral-value normalization must cover it, not only the
/// "no rows" case. `TestItem.value` is nullable (`f64` mapping to a nullable
/// column) and the rows below are inserted with a `NULL` value.
#[tokio::test]
async fn sum_of_all_null_column_returns_zero() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");

    let mut conn = ctx.provider().get_connection().await.expect("connection");
    conn.execute(
        "INSERT INTO test_items (name, value) VALUES ('null-a', NULL), ('null-b', NULL)",
        &[],
    )
    .await
    .expect("insert NULL values");

    let sum: f64 = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; sum b.value)
        .await
        .expect("NULL-only SUM must not error");
    assert_eq!(sum, 0.0);
}
