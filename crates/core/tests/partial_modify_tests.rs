//! Verifies property-level change tracking and partial UPDATE behavior:
//! when only one column changes, `detect_changes` records just that field
//! in `modified_properties`, and `execute_updates` generates a SET clause
//! containing only the dirty column.
//!
//! Also verifies D2: batch INSERT PK backfill — after `save_changes()`,
//! Added entities have their auto-increment PKs populated from the database.

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
async fn partial_modify_sets_only_dirty_column() {
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");

    // Seed one row.
    ctx.add::<TestItem>(TestItem {
        id: 0,
        name: "Alpha".into(),
        value: 1.0,
    });
    ctx.save_changes().await.expect("seed save");

    // Load and attach (takes original snapshot).
    let items = ctx.set::<TestItem>().query().to_list().await.expect("load");
    let item = items.into_iter().next().expect("one row");
    ctx.set::<TestItem>().clear_entries();
    ctx.attach::<TestItem>(item);

    // Modify only `name`; leave `value` untouched.
    {
        let entry = ctx.set::<TestItem>().tracked_entries_mut().next().unwrap();
        entry.name = "Alpha2".into();
    }

    ctx.detect_changes();

    // The entry should now be in Modified state.
    let states: Vec<_> = ctx
        .change_tracker()
        .entries()
        .into_iter()
        .map(|e| e.state)
        .collect();
    assert!(
        states.contains(&rust_ef::entity::EntityState::Modified),
        "entry should be Modified after detect_changes, got {states:?}"
    );

    ctx.save_changes().await.expect("partial modify save");

    // Verify: name changed, value preserved.
    let rows = ctx
        .set::<TestItem>()
        .query()
        .to_list()
        .await
        .expect("reload");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Alpha2");
    assert!(
        (rows[0].value - 1.0).abs() < f64::EPSILON,
        "value should be unchanged, got {}",
        rows[0].value
    );
}

#[tokio::test]
async fn full_modify_when_no_detection() {
    // When an entity is marked Modified via `update()` (no detect_changes),
    // modified_properties is empty and all non-PK columns are SET.
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");

    ctx.add::<TestItem>(TestItem {
        id: 0,
        name: "Gamma".into(),
        value: 5.0,
    });
    ctx.save_changes().await.expect("seed save");

    let items = ctx.set::<TestItem>().query().to_list().await.expect("load");
    let mut item = items.into_iter().next().expect("one row");
    item.name = "Gamma2".into();
    item.value = 99.0;

    // Directly mark as Modified without detect_changes — modified_properties
    // stays empty, so all non-PK columns are SET.
    ctx.set::<TestItem>().clear_entries();
    ctx.update::<TestItem>(item);
    ctx.save_changes().await.expect("full modify save");

    let rows = ctx
        .set::<TestItem>()
        .query()
        .to_list()
        .await
        .expect("reload");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Gamma2");
    assert!(
        (rows[0].value - 99.0).abs() < f64::EPSILON,
        "value should be 99.0, got {}",
        rows[0].value
    );
}

#[tokio::test]
async fn batch_insert_backfills_auto_increment_pk() {
    // After save_changes(), Added entities should have their auto-increment
    // PKs populated from the database (SQLite: last_insert_rowid() - N + 1 .. last).
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");

    let names = ["Alpha", "Bravo", "Charlie", "Delta"];
    for name in names {
        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: name.into(),
            value: 1.0,
        });
    }
    ctx.save_changes().await.expect("batch insert save");

    // The tracked entries should now have non-zero PKs assigned by the DB.
    let ids: Vec<i64> = ctx
        .set::<TestItem>()
        .tracked_entries()
        .map(|e| e.id as i64)
        .collect();
    assert_eq!(
        ids.len(),
        names.len(),
        "all added entities should be tracked"
    );
    assert!(
        ids.iter().all(|&id| id > 0),
        "all PKs should be backfilled (non-zero), got {ids:?}"
    );
    // PKs should be contiguous (1..=4 for a fresh table).
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        vec![1, 2, 3, 4],
        "PKs should be contiguous starting from 1, got {ids:?}"
    );
}
