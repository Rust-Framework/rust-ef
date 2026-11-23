//! EF-02: typed scalar aggregates (`sum_as`, `sum_i64`, `avg_as`).
//!
//! `sum`/`avg` keep their historical `f64` contract (2.x). The typed variants
//! decode the provider cell directly into the caller-chosen type, so integer
//! money/counters stay exact beyond the `f64` 2^53 integer-precision limit.
//! The EF-17 neutral-value rule is preserved: an empty input set (or an
//! all-`NULL` column) yields the target type's zero, never a conversion error.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::query::{BoolExpr, FilterCondition};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

/// 2^53 + 1 — an odd integer that `f64` cannot represent exactly.
const HUGE: i64 = 9_007_199_254_740_993;

#[derive(Debug, Clone, EntityType)]
#[table("typed_amounts")]
struct Amount {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    bucket: String,
    amount: i64,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("context")
}

async fn ctx_with_rows(rows: &[(&str, i64)]) -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<Amount>();
    ctx.ensure_created().await.expect("ensure_created");
    for (bucket, amount) in rows {
        ctx.add::<Amount>(Amount {
            id: 0,
            bucket: bucket.to_string(),
            amount: *amount,
        });
    }
    ctx.save_changes().await.expect("seed save");
    ctx.set::<Amount>().clear_entries();
    ctx
}

fn filter_bucket(bucket: &str) -> BoolExpr {
    BoolExpr::Filter(FilterCondition::with_values(
        "bucket",
        "=",
        vec![DbValue::String(bucket.to_string())],
    ))
}

#[tokio::test]
async fn sum_as_i64_is_exact_beyond_f64_precision() {
    let mut ctx = ctx_with_rows(&[("big", HUGE), ("big", 2)]).await;

    // SQLite SUM over INTEGER inputs stays integer-exact: 2^53 + 3.
    let exact: i64 = ctx
        .set::<Amount>()
        .query()
        .sum_as::<i64>("amount")
        .await
        .expect("typed SUM");
    assert_eq!(exact, HUGE + 2);

    // The historical f64 terminal cannot represent that odd integer.
    let lossy: f64 = ctx
        .set::<Amount>()
        .query()
        .sum_f64("amount")
        .await
        .expect("f64 SUM");
    assert_ne!(
        lossy as i64, exact,
        "f64 SUM loses precision above 2^53: {lossy}"
    );
}

#[tokio::test]
async fn sum_i64_is_the_i64_convenience_form() {
    let mut ctx = ctx_with_rows(&[("a", 1), ("a", 2), ("a", 3)]).await;

    let via_as: i64 = ctx
        .set::<Amount>()
        .query()
        .sum_as::<i64>("amount")
        .await
        .expect("sum_as");
    let via_convenience: i64 = ctx
        .set::<Amount>()
        .query()
        .sum_i64("amount")
        .await
        .expect("sum_i64");

    assert_eq!(via_as, 6);
    assert_eq!(via_convenience, via_as);
}

#[tokio::test]
async fn sum_as_f64_matches_the_legacy_f64_contract() {
    let mut ctx = ctx_with_rows(&[("a", 1), ("a", 2), ("a", 3)]).await;

    let sum_as: f64 = ctx
        .set::<Amount>()
        .query()
        .sum_as::<f64>("amount")
        .await
        .expect("sum_as::<f64>");
    let legacy: f64 = ctx
        .set::<Amount>()
        .query()
        .sum_f64("amount")
        .await
        .expect("sum_internal");

    assert_eq!(sum_as, 6.0);
    assert_eq!(legacy, sum_as);
}

#[tokio::test]
async fn typed_aggregates_respect_filters() {
    let mut ctx = ctx_with_rows(&[("small", 1), ("small", 2), ("other", 100)]).await;

    let sum: i64 = ctx
        .set::<Amount>()
        .query()
        .filter(filter_bucket("small"))
        .sum_as::<i64>("amount")
        .await
        .expect("filtered SUM");
    let avg: f64 = ctx
        .set::<Amount>()
        .query()
        .filter(filter_bucket("small"))
        .avg_as::<f64>("amount")
        .await
        .expect("filtered AVG");

    assert_eq!(sum, 3);
    assert_eq!(avg, 1.5);
}

#[tokio::test]
async fn avg_as_decodes_the_average_as_the_requested_type() {
    let mut ctx = ctx_with_rows(&[("a", 1), ("a", 2), ("a", 3)]).await;

    let avg: f64 = ctx
        .set::<Amount>()
        .query()
        .avg_as::<f64>("amount")
        .await
        .expect("avg_as::<f64>");
    assert_eq!(avg, 2.0);

    // Integer targets accept only integer-shaped cells: SQLite AVG returns a
    // REAL cell, so decoding it as i64 is a typed conversion error.
    let err = ctx
        .set::<Amount>()
        .query()
        .avg_as::<i64>("amount")
        .await
        .expect_err("REAL AVG cell cannot be decoded as i64");
    assert!(
        matches!(err, EFError::TypeConversion(..)),
        "expected TypeConversion, got {err:?}"
    );
}

#[tokio::test]
async fn empty_set_typed_aggregates_are_the_neutral_zero() {
    let mut ctx = make_ctx();
    ctx.set::<Amount>();
    ctx.ensure_created().await.expect("ensure_created");

    let sum: i64 = ctx
        .set::<Amount>()
        .query()
        .sum_as::<i64>("amount")
        .await
        .expect("empty SUM");
    assert_eq!(sum, 0);

    let sum_f64: f64 = ctx
        .set::<Amount>()
        .query()
        .sum_as::<f64>("amount")
        .await
        .expect("empty SUM as f64");
    assert_eq!(sum_f64, 0.0);

    let avg: f64 = ctx
        .set::<Amount>()
        .query()
        .avg_as::<f64>("amount")
        .await
        .expect("empty AVG");
    assert_eq!(avg, 0.0);

    let sum_i64: i64 = ctx
        .set::<Amount>()
        .query()
        .sum_i64("amount")
        .await
        .expect("empty SUM via sum_i64");
    assert_eq!(sum_i64, 0);
}

/// A filtered-to-empty query follows the same neutral-value rule.
#[tokio::test]
async fn filtered_to_empty_typed_aggregates_are_zero() {
    let mut ctx = ctx_with_rows(&[("a", 1)]).await;

    let sum: i64 = ctx
        .set::<Amount>()
        .query()
        .filter(filter_bucket("missing"))
        .sum_i64("amount")
        .await
        .expect("filtered-to-empty SUM");
    assert_eq!(sum, 0);

    let avg: f64 = ctx
        .set::<Amount>()
        .query()
        .filter(filter_bucket("missing"))
        .avg_as::<f64>("amount")
        .await
        .expect("filtered-to-empty AVG");
    assert_eq!(avg, 0.0);
}

#[tokio::test]
async fn min_and_max_keep_their_option_signature() {
    let mut ctx = ctx_with_rows(&[("a", 5), ("a", 9)]).await;

    let min: Option<i64> = ctx
        .set::<Amount>()
        .query()
        .min("amount")
        .await
        .expect("MIN");
    let max: Option<i64> = ctx
        .set::<Amount>()
        .query()
        .max("amount")
        .await
        .expect("MAX");
    assert_eq!(min, Some(5));
    assert_eq!(max, Some(9));

    // `linq!` terminals keep the same behavior (macro regression guard).
    let via_macro: Option<i64> = linq!(ctx.set::<Amount>().query(), |a: Amount| true; min a.amount)
        .await
        .expect("linq MIN");
    assert_eq!(via_macro, Some(5));

    let mut empty = make_ctx();
    empty.set::<Amount>();
    empty.ensure_created().await.expect("ensure_created");
    let empty_min: Option<i64> = empty
        .set::<Amount>()
        .query()
        .min("amount")
        .await
        .expect("empty MIN");
    assert_eq!(empty_min, None);
}

/// A sum that cannot decode into the requested type is a typed conversion error.
#[tokio::test]
async fn sum_as_reports_a_typed_error_for_an_incompatible_target() {
    let mut ctx = ctx_with_rows(&[("a", 7)]).await;

    let err = ctx
        .set::<Amount>()
        .query()
        .sum_as::<Vec<u8>>("amount")
        .await
        .expect_err("an integer SUM cell is not a byte array");
    assert!(
        matches!(err, EFError::TypeConversion(..)),
        "expected TypeConversion, got {err:?}"
    );
}
