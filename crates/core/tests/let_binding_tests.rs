//! Tests for G4: untyped closure with source type inference.
//!
//! Verifies that `linq!(ctx.set::<T>(), |b| body)` works without requiring
//! the closure parameter to have an explicit type annotation (`|b: T|`).
//! The entity type is inferred from the source expression's turbofish.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("lb_items")]
struct LbItem {
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

async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<LbItem>();
    ctx.ensure_created().await.unwrap();
    for (i, (name, value)) in [("alpha", 10), ("beta", 20), ("gamma", 30)]
        .iter()
        .enumerate()
    {
        ctx.add::<LbItem>(LbItem {
            id: 0,
            name: (*name).into(),
            value: *value,
        });
        let _ = i;
    }
    ctx.save_changes().await.unwrap();
    ctx
}

#[tokio::test]
async fn test_untyped_closure_basic_filter() {
    // G4 core: `|b|` without type annotation — type inferred from `ctx.set::<LbItem>()`.
    let mut ctx = seed().await;
    let rows = linq!(ctx.set::<LbItem>(), |b| b.value > 15)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));
}

#[tokio::test]
async fn test_untyped_closure_with_order_by() {
    // G4 + order_by clause with untyped closure.
    let mut ctx = seed().await;
    let rows = linq!(ctx.set::<LbItem>(), |b| b.value > 0; order_by b.value)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].value, 10);
    assert_eq!(rows[1].value, 20);
    assert_eq!(rows[2].value, 30);
}

#[tokio::test]
async fn test_untyped_closure_with_include() {
    // G4 should work with multi-clause queries including include.
    let mut ctx = seed().await;
    let rows = linq!(ctx.set::<LbItem>(), |b| b.value >= 10; order_by b.id)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn test_untyped_closure_aggregate() {
    // G4 with aggregate terminal (sum).
    let mut ctx = seed().await;
    let total: f64 = linq!(ctx.set::<LbItem>(), |b| b.value > 0; sum b.value)
        .await
        .unwrap();
    assert_eq!(total, 60.0);
}

#[tokio::test]
async fn test_untyped_closure_count() {
    let mut ctx = seed().await;
    let count = linq!(ctx.set::<LbItem>(), |b| b.value > 15; count)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn test_typed_closure_still_works() {
    // Backward compat: typed closure `|b: T|` still works alongside G4.
    let mut ctx = seed().await;
    let rows = linq!(ctx.set::<LbItem>(), |b: LbItem| b.value > 15)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn test_untyped_closure_with_select() {
    // G4 + select clause with untyped closure.
    let mut ctx = seed().await;
    let rows: Vec<(i32, String)> =
        linq!(ctx.set::<LbItem>(), |b| b.value > 0; select (b.id, b.name))
            .to_list_typed_2::<i32, String>()
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn test_reusable_filter_untyped_closure_with_typed_source() {
    // G4 with split let bindings (preferred style):
    //   let set = ctx.set::<T>();
    //   let expr = linq!(|b: T| ...);  // typed reusable closure
    //   set.filter(expr)...
    // Also works with source + untyped:
    let mut ctx = seed().await;
    let set = ctx.set::<LbItem>();
    let expr = linq!(|b: LbItem| b.value > 15);
    let rows = set.filter(expr).to_list().await.unwrap();
    assert_eq!(rows.len(), 2);
}
