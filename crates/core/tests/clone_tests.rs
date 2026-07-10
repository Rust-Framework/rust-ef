//! Tests for `QueryBuilder<T>: Clone` (G1) — builder reuse scenarios.
//!
//! Covers:
//! - Cloning a builder preserves accumulated state (filters, ordering, pagination).
//! - Mutating a clone does not affect the original (independent state).
//! - Two clones from the same base can diverge into different queries.
//! - `SelectQueryBuilder<T>: Clone` and `ExecuteUpdateBuilder<T>: Clone`.
//! - `linq!` expression reuse via cloning.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("clone_items")]
struct CloneItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
    value: i32,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Seeds 5 items with values 10, 20, 30, 40, 50.
async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<CloneItem>();
    ctx.ensure_created().await.unwrap();
    for i in 0..5 {
        ctx.add::<CloneItem>(CloneItem {
            id: 0,
            name: format!("i{}", i),
            value: (i + 1) * 10,
        });
    }
    ctx.save_changes().await.unwrap();
    ctx
}

#[tokio::test]
async fn test_query_builder_clone_compiles_and_runs() {
    // G1 core: QueryBuilder<T> implements Clone.
    let mut ctx = seed().await;
    let base = ctx.set::<CloneItem>().query();
    let _copy = base.clone();
    // Original is still usable (clone did not move out of base).
    let rows = base.to_list().await.unwrap();
    assert_eq!(rows.len(), 5);
}

#[tokio::test]
async fn test_clone_preserves_state() {
    // A clone inherits filters, ordering, and pagination from the original.
    let mut ctx = seed().await;
    let base = ctx
        .set::<CloneItem>()
        .query()
        .order_by_column("value")
        .take(3);
    let copy = base.clone();
    // Both should return the same 3 rows (first three by value ascending).
    let original_rows = base.to_list().await.unwrap();
    let cloned_rows = copy.to_list().await.unwrap();
    assert_eq!(original_rows.len(), 3);
    assert_eq!(cloned_rows.len(), 3);
    assert_eq!(original_rows[0].value, 10);
    assert_eq!(cloned_rows[0].value, 10);
}

#[tokio::test]
async fn test_clone_divergence_independent_state() {
    // Modifying a clone must not affect the original builder's state.
    let mut ctx = seed().await;
    let base = ctx.set::<CloneItem>().query();
    let high = base.clone().filter_column("value", ">", 25);
    let low = base.clone().filter_column("value", "<", 25);
    // Original base is unmodified — returns all 5 rows.
    let all = base.to_list().await.unwrap();
    assert_eq!(all.len(), 5);
    // high clone: value > 25 → 30, 40, 50 (3 rows)
    let high_rows = high.to_list().await.unwrap();
    assert_eq!(high_rows.len(), 3);
    // low clone: value < 25 → 10, 20 (2 rows)
    let low_rows = low.to_list().await.unwrap();
    assert_eq!(low_rows.len(), 2);
}

#[tokio::test]
async fn test_clone_with_linq_filter_reuse() {
    // Realistic scenario: build a base query with linq!, clone it for
    // different thresholds without re-stating the source.
    let mut ctx = seed().await;
    let set = ctx.set::<CloneItem>();
    let base_expr = linq!(|c: CloneItem| c.value > 0);
    let base = set.filter(base_expr);
    let threshold_high = base.clone();
    let _threshold_low = base.clone();
    // The clone can be further refined.
    let refined = threshold_high.filter(linq!(|c: CloneItem| c.value > 25));
    let rows = refined.to_list().await.unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn test_select_query_builder_clone() {
    // G1 extends to SelectQueryBuilder<T>.
    let mut ctx = seed().await;
    let select = ctx
        .set::<CloneItem>()
        .query()
        .select_internal(&[CloneItem::COLUMN_ID, CloneItem::COLUMN_VALUE]);
    let _copy = select.clone();
    let rows = select.to_list().await.unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].len(), 2);
}

#[tokio::test]
async fn test_execute_update_builder_clone() {
    // G1 extends to ExecuteUpdateBuilder<T>.
    let mut ctx = seed().await;
    let builder = ctx
        .set::<CloneItem>()
        .query()
        .filter_column("value", ">", 30)
        .execute_update()
        .set_column_internal(CloneItem::COLUMN_VALUE, 999);
    let _copy = builder.clone();
    let updated = builder.execute().await.unwrap();
    assert_eq!(updated, 2);
    // Verify the update took effect.
    let remaining = ctx
        .set::<CloneItem>()
        .query()
        .filter_column("value", "=", 999)
        .to_list()
        .await
        .unwrap();
    assert_eq!(remaining.len(), 2);
}
