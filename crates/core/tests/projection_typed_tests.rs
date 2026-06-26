//! Tests for G3: strongly-typed tuple projection via `to_list_typed_n`.
//!
//! Verifies that `SelectQueryBuilder::to_list_typed_1..4` correctly
//! parses raw string columns into typed Rust values and returns tuples.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("proj_items")]
struct ProjItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
    value: i32,
    active: bool,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Seeds 3 items.
async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<ProjItem>();
    ctx.ensure_created().await.unwrap();
    ctx.set::<ProjItem>().add(ProjItem {
        id: 0,
        name: "alpha".into(),
        value: 100,
        active: true,
    });
    ctx.set::<ProjItem>().add(ProjItem {
        id: 0,
        name: "beta".into(),
        value: 200,
        active: false,
    });
    ctx.set::<ProjItem>().add(ProjItem {
        id: 0,
        name: "gamma".into(),
        value: 300,
        active: true,
    });
    ctx.save_changes().await.unwrap();
    ctx
}

#[tokio::test]
async fn test_field_type_constants() {
    // G3.1: FIELD_TYPE_* constants exist and contain type names.
    assert_eq!(ProjItem::FIELD_TYPE_ID, "i32");
    assert_eq!(ProjItem::FIELD_TYPE_NAME, "String");
    assert_eq!(ProjItem::FIELD_TYPE_VALUE, "i32");
    assert_eq!(ProjItem::FIELD_TYPE_ACTIVE, "bool");
}

#[tokio::test]
async fn test_to_list_typed_1() {
    let mut ctx = seed().await;
    let rows: Vec<i32> = ctx
        .set::<ProjItem>()
        .query()
        .select_internal(&[ProjItem::COLUMN_VALUE])
        .to_list_typed_1::<i32>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.contains(&100));
    assert!(rows.contains(&200));
    assert!(rows.contains(&300));
}

#[tokio::test]
async fn test_to_list_typed_2() {
    let mut ctx = seed().await;
    let rows: Vec<(i32, String)> = ctx
        .set::<ProjItem>()
        .query()
        .order_by_column("id")
        .select_internal(&[ProjItem::COLUMN_ID, ProjItem::COLUMN_NAME])
        .to_list_typed_2::<i32, String>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].1, "alpha");
    assert_eq!(rows[1].1, "beta");
    assert_eq!(rows[2].1, "gamma");
}

#[tokio::test]
async fn test_to_list_typed_3() {
    let mut ctx = seed().await;
    let rows: Vec<(i32, String, i32)> = ctx
        .set::<ProjItem>()
        .query()
        .order_by_column("id")
        .select_internal(&[
            ProjItem::COLUMN_ID,
            ProjItem::COLUMN_NAME,
            ProjItem::COLUMN_VALUE,
        ])
        .to_list_typed_3::<i32, String, i32>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], (1, "alpha".into(), 100));
    assert_eq!(rows[1], (2, "beta".into(), 200));
    assert_eq!(rows[2], (3, "gamma".into(), 300));
}

#[tokio::test]
async fn test_to_list_typed_4() {
    let mut ctx = seed().await;
    let rows: Vec<(i32, String, i32, bool)> = ctx
        .set::<ProjItem>()
        .query()
        .order_by_column("id")
        .select_internal(&[
            ProjItem::COLUMN_ID,
            ProjItem::COLUMN_NAME,
            ProjItem::COLUMN_VALUE,
            ProjItem::COLUMN_ACTIVE,
        ])
        .to_list_typed_4::<i32, String, i32, bool>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], (1, "alpha".into(), 100, true));
    assert_eq!(rows[1], (2, "beta".into(), 200, false));
    assert_eq!(rows[2], (3, "gamma".into(), 300, true));
}

#[tokio::test]
async fn test_typed_projection_with_filter() {
    let mut ctx = seed().await;
    let rows: Vec<(String, i32)> = ctx
        .set::<ProjItem>()
        .query()
        .filter_column("value", ">", 150)
        .select_internal(&[ProjItem::COLUMN_NAME, ProjItem::COLUMN_VALUE])
        .to_list_typed_2::<String, i32>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let names: Vec<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"beta"));
    assert!(names.contains(&"gamma"));
}

#[tokio::test]
async fn test_typed_projection_via_linq_select() {
    // Integration with linq! macro's `select` clause.
    let mut ctx = seed().await;
    let rows: Vec<(i32, String)> =
        linq!(ctx.set::<ProjItem>(), |p: ProjItem| p.value > 0; select (p.id, p.name))
            .to_list_typed_2::<i32, String>()
            .await
            .unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn test_typed_projection_with_aggregate() {
    // Aggregate projection returns a single value — typed_1 should work.
    let mut ctx = seed().await;
    let rows: Vec<i64> = ctx
        .set::<ProjItem>()
        .query()
        .select_internal(&["COUNT(*)"])
        .to_list_typed_1::<i64>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0], 3);
}
