//! Integration tests for optimistic concurrency control using
//! `#[derive(EntityType)]` + `#[concurrency_check]`.
//!
//! Verifies end-to-end that the macro correctly emits `is_concurrency_token`
//! metadata and that `ChangeExecutor` includes token columns in UPDATE/DELETE
//! WHERE clauses, returning `ConcurrencyConflict` when 0 rows are affected.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::IEntityType;
use rust_ef::error::EFError;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("conc_items")]
struct ConcItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
    #[concurrency_check]
    row_version: i32,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Inserts one `ConcItem { name: "alpha", row_version: 1 }`, then reloads it.
/// Returns `(ctx, loaded_item)`.
async fn seed_one() -> (DbContext, ConcItem) {
    let mut ctx = make_ctx();
    ctx.set::<ConcItem>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<ConcItem>(ConcItem {
        id: 0,
        name: "alpha".into(),
        row_version: 1,
    });
    ctx.save_changes().await.unwrap();

    let items = ctx.set::<ConcItem>().query().to_list().await.unwrap();
    let item = items.into_iter().next().unwrap();
    (ctx, item)
}

#[test]
fn test_macro_concurrency_token_meta() {
    let meta = ConcItem::entity_meta();
    let token_prop = meta
        .properties
        .iter()
        .find(|p| p.field_name.as_ref() == "row_version")
        .expect("row_version property should exist");
    assert!(
        token_prop.is_concurrency_token,
        "row_version should be a concurrency token"
    );

    let name_prop = meta
        .properties
        .iter()
        .find(|p| p.field_name.as_ref() == "name")
        .unwrap();
    assert!(
        !name_prop.is_concurrency_token,
        "name should NOT be a concurrency token"
    );
}

#[tokio::test]
async fn test_update_conflict_stale_token() {
    let (mut ctx, item) = seed_one().await;

    ctx.set::<ConcItem>().clear_entries();
    ctx.attach::<ConcItem>(item);

    // Modify the entity — bump name and row_version (app-level token increment)
    ctx.set::<ConcItem>()
        .tracked_entries_mut()
        .next()
        .unwrap()
        .name = "beta".into();

    // Simulate another writer bumping row_version in the database.
    let mut conn = ctx.provider().get_connection().await.unwrap();
    conn.execute("UPDATE conc_items SET row_version = 99 WHERE id = 1", &[])
        .await
        .unwrap();

    ctx.detect_changes();
    let result = ctx.save_changes().await;
    assert!(
        matches!(result, Err(EFError::ConcurrencyConflict(..))),
        "expected ConcurrencyConflict, got {result:?}"
    );
}

#[tokio::test]
async fn test_delete_conflict_stale_token() {
    let (mut ctx, item) = seed_one().await;

    ctx.set::<ConcItem>().clear_entries();
    ctx.attach::<ConcItem>(item);
    ctx.remove_at::<ConcItem>(0).unwrap();

    // Simulate another writer bumping row_version in the database.
    let mut conn = ctx.provider().get_connection().await.unwrap();
    conn.execute("UPDATE conc_items SET row_version = 99 WHERE id = 1", &[])
        .await
        .unwrap();

    let result = ctx.save_changes().await;
    assert!(
        matches!(result, Err(EFError::ConcurrencyConflict(..))),
        "expected ConcurrencyConflict on delete, got {result:?}"
    );
}

#[tokio::test]
async fn test_update_succeeds_no_concurrent_modification() {
    let (mut ctx, item) = seed_one().await;

    ctx.set::<ConcItem>().clear_entries();
    ctx.attach::<ConcItem>(item);

    // Modify name and increment row_version (app-level token strategy).
    {
        let entry = ctx.set::<ConcItem>().tracked_entries_mut().next().unwrap();
        entry.name = "gamma".into();
        entry.row_version += 1;
    }

    ctx.detect_changes();
    ctx.save_changes().await.unwrap();

    // Verify the update persisted.
    let rows = ctx.set::<ConcItem>().query().to_list().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "gamma");
    assert_eq!(rows[0].row_version, 2);
}

#[tokio::test]
async fn test_delete_succeeds_no_concurrent_modification() {
    let (mut ctx, item) = seed_one().await;

    ctx.set::<ConcItem>().clear_entries();
    ctx.attach::<ConcItem>(item);
    ctx.remove_at::<ConcItem>(0).unwrap();

    ctx.save_changes().await.unwrap();

    let rows = ctx.set::<ConcItem>().query().to_list().await.unwrap();
    assert!(rows.is_empty(), "entity should have been deleted");
}

#[tokio::test]
async fn test_update_after_token_refresh() {
    // After a conflict, re-loading the entity gives the fresh token.
    // A subsequent update with the fresh token should succeed.
    let (mut ctx, item) = seed_one().await;

    ctx.set::<ConcItem>().clear_entries();
    ctx.attach::<ConcItem>(item);
    ctx.set::<ConcItem>()
        .tracked_entries_mut()
        .next()
        .unwrap()
        .name = "beta".into();

    // Concurrent writer bumps row_version.
    let mut conn = ctx.provider().get_connection().await.unwrap();
    conn.execute("UPDATE conc_items SET row_version = 99 WHERE id = 1", &[])
        .await
        .unwrap();

    ctx.detect_changes();
    let result = ctx.save_changes().await;
    assert!(matches!(result, Err(EFError::ConcurrencyConflict(..))));

    // Re-load with fresh token and retry.
    ctx.set::<ConcItem>().clear_entries();
    let fresh = ctx
        .set::<ConcItem>()
        .query()
        .to_list()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(fresh.row_version, 99, "fresh load should see bumped token");

    ctx.attach::<ConcItem>(fresh);
    {
        let entry = ctx.set::<ConcItem>().tracked_entries_mut().next().unwrap();
        entry.name = "delta".into();
        entry.row_version += 1; // 99 → 100
    }
    ctx.detect_changes();
    ctx.save_changes().await.unwrap();

    let rows = ctx.set::<ConcItem>().query().to_list().await.unwrap();
    assert_eq!(rows[0].name, "delta");
    assert_eq!(rows[0].row_version, 100);
}
