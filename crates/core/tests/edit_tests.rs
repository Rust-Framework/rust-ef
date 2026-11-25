//! `DbContext::edit` — attach-with-snapshot-then-mutate, so the "load detached
//! → modify → save" path is tracked.
//!
//! A detached entity cannot be modified *before* `attach()`: `attach` snapshots
//! the values it receives, so `detect_changes()` sees no difference. `edit`
//! snapshots first, then runs the closure on the tracked copy.

mod common;

use common::TestItem;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::EntityState;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    DbContext::from_options(&builder.build()).expect("ctx")
}

/// Seeds one row, then drops all tracking state and loads the row through a
/// query terminal (which does not track) — a genuinely detached entity.
async fn seed_and_load_detached(ctx: &mut DbContext) -> TestItem {
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx.add::<TestItem>(TestItem {
        id: 0,
        name: "Alpha".into(),
        value: 1.0,
    });
    ctx.save_changes().await.expect("seed save");
    ctx.clear_tracking();

    let mut items = ctx.set::<TestItem>().query().to_list().await.expect("load");
    assert_eq!(items.len(), 1, "seed must produce exactly one row");
    items.remove(0)
}

#[tokio::test]
async fn edit_snapshots_before_the_closure_so_only_its_changes_are_written() {
    let mut ctx = make_ctx();
    let mut item = seed_and_load_detached(&mut ctx).await;

    // Changing the detached instance before `edit` behaves like changing it
    // before `attach`: the value is part of the snapshot and is not written.
    item.value = 99.0;

    ctx.edit(item, |tracked| tracked.name = "Beta".into());

    // edit() neither detects nor saves — the caller owns the transaction.
    assert!(
        !ctx.change_tracker().has_changes(),
        "edit must not run detect_changes"
    );
    let states: Vec<EntityState> = ctx
        .change_tracker()
        .entries()
        .iter()
        .map(|e| e.state)
        .collect();
    assert_eq!(states, vec![EntityState::Unchanged]);

    let stored = ctx.set::<TestItem>().query().to_list().await.expect("read");
    assert_eq!(stored[0].name, "Alpha", "edit must not save");
    assert!((stored[0].value - 1.0).abs() < f64::EPSILON);

    // The snapshot predates the closure, so only `name` is dirty.
    ctx.detect_changes();
    let entries = ctx.change_tracker().entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].state, EntityState::Modified);
    assert_eq!(
        entries[0].modified_properties,
        vec!["name".to_string()],
        "only the column changed by the closure may be marked dirty"
    );

    ctx.save_changes().await.expect("save edit");

    let reloaded = ctx
        .set::<TestItem>()
        .query()
        .to_list()
        .await
        .expect("reload");
    assert_eq!(reloaded[0].name, "Beta");
    assert!(
        (reloaded[0].value - 1.0).abs() < f64::EPSILON,
        "only `name` may be UPDATEd; the stale pre-edit value must not be written"
    );
}

#[tokio::test]
async fn edit_works_on_a_reloaded_entity_without_prior_attach() {
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx.add::<TestItem>(TestItem {
        id: 0,
        name: "Alpha".into(),
        value: 1.0,
    });
    ctx.save_changes().await.expect("seed save");
    ctx.clear_tracking();

    let item = ctx
        .set::<TestItem>()
        .query()
        .find(1i32)
        .await
        .expect("find")
        .expect("seeded row");

    ctx.edit(item, |tracked| {
        tracked.name = "Renamed".into();
        tracked.value = 7.5;
    });
    ctx.detect_changes();
    ctx.save_changes().await.expect("save edit");

    let reloaded = ctx
        .set::<TestItem>()
        .query()
        .find(1i32)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(reloaded.name, "Renamed");
    assert!((reloaded.value - 7.5).abs() < f64::EPSILON);
}

#[tokio::test]
async fn edit_reuses_the_tracked_entry_instead_of_duplicating_it() {
    let mut ctx = make_ctx();
    let first = seed_and_load_detached(&mut ctx).await;
    ctx.edit(first, |tracked| tracked.name = "First edit".into());

    // A second detached copy of the same row: a plain `attach` would create a
    // second DbSet/tracker entry for the same key.
    let second = ctx
        .set::<TestItem>()
        .query()
        .find(1i32)
        .await
        .expect("find")
        .expect("seeded row");

    ctx.edit(second, |tracked| tracked.value = 42.0);

    assert_eq!(ctx.set::<TestItem>().len(), 1, "one DbSet entry per row");
    assert_eq!(
        ctx.change_tracker().entries().len(),
        1,
        "one tracker entry per row"
    );

    ctx.detect_changes();
    let entries = ctx.change_tracker().entries();
    assert_eq!(
        entries[0].modified_properties,
        vec!["name".to_string(), "value".to_string()],
        "both edits land on the same tracked entry"
    );

    ctx.save_changes().await.expect("save edit");
    let reloaded = ctx
        .set::<TestItem>()
        .query()
        .find(1i32)
        .await
        .expect("find")
        .expect("row");
    assert_eq!(
        reloaded.name, "First edit",
        "the tracked instance must win over the stale copy"
    );
    assert!((reloaded.value - 42.0).abs() < f64::EPSILON);
}
