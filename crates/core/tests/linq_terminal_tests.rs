//! Tests for the LINQ terminal methods on `QueryBuilder<T>`:
//! `last`, `last_or_default`, `single`, `single_or_default`, `to_dictionary`,
//! `distinct`, `all`, `contains`, `long_count`.
//!
//! Covers empty / single / multi element boundaries.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::collections::HashMap;

#[derive(Debug, Clone, EntityType)]
#[table("term_items")]
struct TermItem {
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

/// Seeds `count` items with values 10, 20, 30, ... and names "i0", "i1", ...
async fn seed(count: usize) -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<TermItem>();
    ctx.ensure_created().await.unwrap();
    for i in 0..count {
        ctx.set::<TermItem>().add(TermItem {
            id: 0,
            name: format!("i{}", i),
            value: (i as i32 + 1) * 10,
        });
    }
    ctx.save_changes().await.unwrap();
    ctx
}

#[tokio::test]
async fn test_last_returns_entity() {
    let mut ctx = seed(3).await;
    let last = ctx.set::<TermItem>().query().last().await.unwrap();
    // Default ordering by PK ascending → last is the one with highest id.
    assert_eq!(last.name, "i2");
}

#[tokio::test]
async fn test_last_errors_on_empty() {
    let mut ctx = make_ctx();
    ctx.set::<TermItem>();
    ctx.ensure_created().await.unwrap();
    let result = ctx.set::<TermItem>().query().last().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_last_or_default_none_on_empty() {
    let mut ctx = make_ctx();
    ctx.set::<TermItem>();
    ctx.ensure_created().await.unwrap();
    let result = ctx.set::<TermItem>().query().last_or_default().await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_last_or_default_some_on_nonempty() {
    let mut ctx = seed(2).await;
    let result = ctx.set::<TermItem>().query().last_or_default().await.unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().name, "i1");
}

#[tokio::test]
async fn test_single_returns_one() {
    let mut ctx = seed(1).await;
    let item = ctx.set::<TermItem>().query().single().await.unwrap();
    assert_eq!(item.name, "i0");
}

#[tokio::test]
async fn test_single_errors_on_empty() {
    let mut ctx = make_ctx();
    ctx.set::<TermItem>();
    ctx.ensure_created().await.unwrap();
    let result = ctx.set::<TermItem>().query().single().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_single_errors_on_multiple() {
    let mut ctx = seed(3).await;
    let result = ctx.set::<TermItem>().query().single().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_single_or_default_none_on_empty() {
    let mut ctx = make_ctx();
    ctx.set::<TermItem>();
    ctx.ensure_created().await.unwrap();
    let result = ctx.set::<TermItem>().query().single_or_default().await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_single_or_default_some_on_one() {
    let mut ctx = seed(1).await;
    let result = ctx.set::<TermItem>().query().single_or_default().await.unwrap();
    assert!(result.is_some());
}

#[tokio::test]
async fn test_single_or_default_errors_on_multiple() {
    let mut ctx = seed(3).await;
    let result = ctx.set::<TermItem>().query().single_or_default().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_to_dictionary_by_id() {
    let mut ctx = seed(3).await;
    let map: HashMap<i32, TermItem> = ctx
        .set::<TermItem>()
        .query()
        .to_dictionary(|b| b.id)
        .await
        .unwrap();
    assert_eq!(map.len(), 3);
    // Keys are the PKs; find the item named "i1".
    let target = map.values().find(|b| b.name == "i1").unwrap();
    assert_eq!(target.value, 20);
}

#[tokio::test]
async fn test_distinct_method() {
    let mut ctx = seed(3).await;
    let items = ctx
        .set::<TermItem>()
        .query()
        .distinct()
        .to_list()
        .await
        .unwrap();
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn test_all_true_when_all_match() {
    let mut ctx = seed(3).await;
    let all_positive = ctx
        .set::<TermItem>()
        .query()
        .all(|b| b.value > 0)
        .await
        .unwrap();
    assert!(all_positive);
}

#[tokio::test]
async fn test_all_false_when_some_dont_match() {
    let mut ctx = seed(3).await;
    let all_big = ctx
        .set::<TermItem>()
        .query()
        .all(|b| b.value > 20)
        .await
        .unwrap();
    // i0 has value 10, which is not > 20.
    assert!(!all_big);
}

#[tokio::test]
async fn test_all_true_on_empty() {
    let mut ctx = make_ctx();
    ctx.set::<TermItem>();
    ctx.ensure_created().await.unwrap();
    let all_positive = ctx
        .set::<TermItem>()
        .query()
        .all(|b| b.value > 0)
        .await
        .unwrap();
    // Vacuously true.
    assert!(all_positive);
}

#[tokio::test]
async fn test_contains_existing_pk() {
    let mut ctx = seed(3).await;
    let exists = ctx.set::<TermItem>().query().contains(1).await.unwrap();
    assert!(exists);
}

#[tokio::test]
async fn test_contains_missing_pk() {
    let mut ctx = seed(3).await;
    let exists = ctx.set::<TermItem>().query().contains(999).await.unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn test_long_count_matches_count() {
    let mut ctx = seed(3).await;
    let count = ctx.set::<TermItem>().query().count().await.unwrap();
    let long_count = ctx.set::<TermItem>().query().long_count().await.unwrap();
    assert_eq!(count, long_count);
    assert_eq!(long_count, 3);
}
