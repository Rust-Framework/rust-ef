//! Verifies D3: Upsert API (`DbSet::upsert`).
//!
//! - First upsert with a new PK → INSERT
//! - Second upsert with the same PK → UPDATE (ON CONFLICT DO UPDATE)
//! - Batch upsert with mixed new/existing PKs

mod common;

use common::TestItem;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("ctx")
}

#[tokio::test]
async fn upsert_inserts_then_updates_on_conflict() {
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");

    // First upsert: id=1 doesn't exist → INSERT.
    ctx.set::<TestItem>().upsert(TestItem {
        id: 1,
        name: "Alpha".into(),
        value: 1.0,
    });
    ctx.save_changes().await.expect("first upsert save");

    let rows = ctx.set::<TestItem>().query().to_list().await.expect("load");
    assert_eq!(rows.len(), 1, "one row after first upsert");
    assert_eq!(rows[0].id, 1);
    assert_eq!(rows[0].name, "Alpha");

    // Clear tracker so the second upsert doesn't conflict with tracked entries.
    ctx.set::<TestItem>().clear_entries();

    // Second upsert: id=1 exists → UPDATE via ON CONFLICT.
    ctx.set::<TestItem>().upsert(TestItem {
        id: 1,
        name: "Alpha2".into(),
        value: 2.0,
    });
    ctx.save_changes().await.expect("second upsert save");

    let rows = ctx
        .set::<TestItem>()
        .query()
        .to_list()
        .await
        .expect("reload");
    assert_eq!(rows.len(), 1, "still one row after upsert update");
    assert_eq!(rows[0].name, "Alpha2", "name should be updated");
    assert!(
        (rows[0].value - 2.0).abs() < f64::EPSILON,
        "value should be updated to 2.0, got {}",
        rows[0].value
    );
}

#[tokio::test]
async fn batch_upsert_mixed_new_and_existing() {
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");

    // Seed id=1 via a regular add.
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Seed".into(),
        value: 0.0,
    });
    ctx.save_changes().await.expect("seed save");
    ctx.set::<TestItem>().clear_entries();

    // Batch upsert: id=1 (exists → UPDATE), id=2 (new → INSERT), id=3 (new → INSERT).
    ctx.set::<TestItem>().upsert(TestItem {
        id: 1,
        name: "Updated".into(),
        value: 10.0,
    });
    ctx.set::<TestItem>().upsert(TestItem {
        id: 2,
        name: "New2".into(),
        value: 20.0,
    });
    ctx.set::<TestItem>().upsert(TestItem {
        id: 3,
        name: "New3".into(),
        value: 30.0,
    });
    ctx.save_changes().await.expect("batch upsert save");

    let mut rows = ctx
        .set::<TestItem>()
        .query()
        .to_list()
        .await
        .expect("reload");
    rows.sort_by_key(|r| r.id);
    assert_eq!(rows.len(), 3, "three rows total after batch upsert");
    assert_eq!(rows[0].id, 1);
    assert_eq!(rows[0].name, "Updated", "id=1 should be updated");
    assert_eq!(rows[1].id, 2);
    assert_eq!(rows[1].name, "New2", "id=2 should be inserted");
    assert_eq!(rows[2].id, 3);
    assert_eq!(rows[2].name, "New3", "id=3 should be inserted");
}
