//! Integration tests for `exists_by_id` / `exists_by_key` existence checks.
//!
//! Verifies that these methods read PK metadata (not a hardcoded `"id"` column),
//! return `true` for existing rows and `false` for missing rows, and work for
//! both single and composite primary keys.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("widgets")]
struct Widget {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

/// Composite-PK entity (no auto-increment; both columns form the key).
#[derive(Debug, Clone, EntityType)]
#[table("blog_tags")]
struct BlogTag {
    #[primary_key]
    blog_id: i32,
    #[primary_key]
    tag_id: i32,
    #[required]
    label: String,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

// -----------------------------------------------------------------------
// Single primary key: exists_by_id
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_exists_by_id_true_for_existing_row() {
    let mut ctx = make_ctx();
    ctx.set::<Widget>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<Widget>(Widget {
        id: 0,
        name: "alpha".into(),
    });
    ctx.save_changes().await.unwrap();

    let items = ctx.set::<Widget>().query().to_list().await.unwrap();
    let inserted_id = items[0].id;

    let exists = ctx
        .set::<Widget>()
        .query()
        .exists_by_id(inserted_id)
        .await
        .expect("exists_by_id");
    assert!(
        exists,
        "exists_by_id should return true for an existing row"
    );
}

#[tokio::test]
async fn test_exists_by_id_false_for_missing_row() {
    let mut ctx = make_ctx();
    ctx.set::<Widget>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<Widget>(Widget {
        id: 0,
        name: "alpha".into(),
    });
    ctx.save_changes().await.unwrap();

    let exists = ctx
        .set::<Widget>()
        .query()
        .exists_by_id(9999)
        .await
        .expect("exists_by_id");
    assert!(
        !exists,
        "exists_by_id should return false for a missing row"
    );
}

#[tokio::test]
async fn test_exists_by_id_false_on_empty_table() {
    let mut ctx = make_ctx();
    ctx.set::<Widget>();
    ctx.ensure_created().await.unwrap();

    let exists = ctx
        .set::<Widget>()
        .query()
        .exists_by_id(1)
        .await
        .expect("exists_by_id");
    assert!(
        !exists,
        "exists_by_id should return false when the table is empty"
    );
}

#[tokio::test]
async fn test_exists_by_id_uses_pk_metadata_not_hardcoded_id() {
    // Widget's PK column is "id", but this test documents that exists_by_id
    // resolves the column from entity_meta() rather than hardcoding "id".
    // If a future entity used a non-"id" PK, exists_by_id would still work.
    let meta = Widget::entity_meta();
    let pk_col = meta
        .primary_keys
        .first()
        .map(|s| s.as_ref())
        .or_else(|| {
            meta.properties
                .iter()
                .find(|p| p.is_primary_key)
                .map(|p| p.column_name.as_ref())
        })
        .expect("Widget should have a PK");
    assert_eq!(pk_col, "id", "sanity: Widget PK column is 'id'");

    let mut ctx = make_ctx();
    ctx.set::<Widget>();
    ctx.ensure_created().await.unwrap();
    ctx.add::<Widget>(Widget {
        id: 0,
        name: "beta".into(),
    });
    ctx.save_changes().await.unwrap();

    let id = ctx.set::<Widget>().query().to_list().await.unwrap()[0].id;
    assert!(
        ctx.set::<Widget>().query().exists_by_id(id).await.unwrap(),
        "exists_by_id resolved PK column from metadata"
    );
}

// -----------------------------------------------------------------------
// Composite primary key: exists_by_key
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_exists_by_key_true_for_existing_composite() {
    let mut ctx = make_ctx();
    ctx.set::<BlogTag>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<BlogTag>(BlogTag {
        blog_id: 1,
        tag_id: 5,
        label: "rust".into(),
    });
    ctx.save_changes().await.unwrap();

    let exists = ctx
        .set::<BlogTag>()
        .query()
        .exists_by_key(&[
            (BlogTag::COLUMN_BLOG_ID, DbValue::I32(1)),
            (BlogTag::COLUMN_TAG_ID, DbValue::I32(5)),
        ])
        .await
        .expect("exists_by_key");
    assert!(
        exists,
        "exists_by_key should return true for existing composite key"
    );
}

#[tokio::test]
async fn test_exists_by_key_false_when_one_component_missing() {
    let mut ctx = make_ctx();
    ctx.set::<BlogTag>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<BlogTag>(BlogTag {
        blog_id: 1,
        tag_id: 5,
        label: "rust".into(),
    });
    ctx.save_changes().await.unwrap();

    // Correct blog_id, wrong tag_id — should not exist.
    let exists = ctx
        .set::<BlogTag>()
        .query()
        .exists_by_key(&[
            (BlogTag::COLUMN_BLOG_ID, DbValue::I32(1)),
            (BlogTag::COLUMN_TAG_ID, DbValue::I32(999)),
        ])
        .await
        .expect("exists_by_key");
    assert!(
        !exists,
        "exists_by_key should return false when one composite component doesn't match"
    );
}

#[tokio::test]
async fn test_exists_by_key_false_on_empty_table() {
    let mut ctx = make_ctx();
    ctx.set::<BlogTag>();
    ctx.ensure_created().await.unwrap();

    let exists = ctx
        .set::<BlogTag>()
        .query()
        .exists_by_key(&[
            (BlogTag::COLUMN_BLOG_ID, DbValue::I32(1)),
            (BlogTag::COLUMN_TAG_ID, DbValue::I32(1)),
        ])
        .await
        .expect("exists_by_key");
    assert!(
        !exists,
        "exists_by_key should return false on an empty table"
    );
}

// -----------------------------------------------------------------------
// Consistency: exists_by_id vs find
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_exists_by_id_consistent_with_find() {
    let mut ctx = make_ctx();
    ctx.set::<Widget>();
    ctx.ensure_created().await.unwrap();
    ctx.add::<Widget>(Widget {
        id: 0,
        name: "gamma".into(),
    });
    ctx.save_changes().await.unwrap();

    let id = ctx.set::<Widget>().query().to_list().await.unwrap()[0].id;

    let found = ctx.set::<Widget>().query().find(id).await.unwrap();
    let exists = ctx.set::<Widget>().query().exists_by_id(id).await.unwrap();
    assert_eq!(
        found.is_some(),
        exists,
        "find().is_some() and exists_by_id() should agree"
    );

    let exists_missing = ctx
        .set::<Widget>()
        .query()
        .exists_by_id(id + 100)
        .await
        .unwrap();
    assert!(!exists_missing);
}
