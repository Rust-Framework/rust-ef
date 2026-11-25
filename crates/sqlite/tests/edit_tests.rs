//! `DbContext::edit` against a real SQLite database.
//!
//! Loads a row through a query terminal (detached), edits it through the
//! tracked copy, lets `detect_changes()` mark the dirty column, and saves. The
//! generated UPDATE must not overwrite the other columns.

use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("edit_orders")]
struct EditOrder {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    #[max_length(64)]
    status: String,
    note: String,
    quantity: i32,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build()).expect("ctx");
    ctx.set::<EditOrder>();
    ctx
}

async fn load(ctx: &mut DbContext, id: i32) -> EditOrder {
    ctx.set::<EditOrder>()
        .query()
        .find(id)
        .await
        .expect("find")
        .expect("row")
}

#[tokio::test]
async fn edit_updates_only_the_column_changed_by_the_closure() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");
    ctx.add(EditOrder {
        id: 0,
        status: "pending".into(),
        note: "keep me".into(),
        quantity: 3,
    });
    ctx.save_changes().await.expect("seed");
    ctx.clear_tracking();

    let mut order = load(&mut ctx, 1).await;
    // Detached mutation made before `edit`: it belongs to the attach snapshot
    // and therefore must never reach the database.
    order.quantity = 99;

    ctx.edit(order, |tracked| tracked.status = "paid".into());

    let entries = ctx.change_tracker().entries();
    assert_eq!(entries.len(), 1, "one tracked entry");
    assert_eq!(entries[0].state, EntityState::Unchanged);

    ctx.detect_changes();
    let entries = ctx.change_tracker().entries();
    assert_eq!(entries[0].state, EntityState::Modified);
    assert_eq!(
        entries[0].modified_properties,
        vec!["status".to_string()],
        "only the closure column may be dirty"
    );

    ctx.save_changes().await.expect("save edit");

    let stored = load(&mut ctx, 1).await;
    assert_eq!(stored.status, "paid");
    assert_eq!(stored.note, "keep me", "only `status` may be UPDATEd");
    assert_eq!(
        stored.quantity, 3,
        "the stale pre-edit value must not be written"
    );
}

#[tokio::test]
async fn edit_reuses_the_tracked_row_on_a_second_load() {
    let mut ctx = make_ctx();
    ctx.ensure_created().await.expect("ensure_created");
    ctx.add(EditOrder {
        id: 0,
        status: "pending".into(),
        note: "keep me".into(),
        quantity: 3,
    });
    ctx.save_changes().await.expect("seed");
    ctx.clear_tracking();

    // First load is attached explicitly; the second detached copy of the same
    // row goes through `edit` and must not create a second entry.
    let first = load(&mut ctx, 1).await;
    ctx.attach(first);
    let second = load(&mut ctx, 1).await;

    ctx.edit(second, |tracked| tracked.quantity = 7);

    assert_eq!(ctx.set::<EditOrder>().len(), 1, "one DbSet entry");
    assert_eq!(ctx.change_tracker().entries().len(), 1, "one tracker entry");

    ctx.detect_changes();
    ctx.save_changes().await.expect("save edit");

    let stored = load(&mut ctx, 1).await;
    assert_eq!(stored.quantity, 7);
    assert_eq!(stored.status, "pending", "the tracked instance wins");
}
