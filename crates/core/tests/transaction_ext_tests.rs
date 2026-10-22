//! Integration tests for the ITransaction abstraction:
//! - `begin_transaction()` returns a scoped `Box<dyn ITransaction>` handle.
//! - `use_transaction()` commits on `Ok`, rolls back on `Err`.
//! - Savepoints: `create_point` / `release_point` / `rollback_point` on the
//!   manual handle.
//! - Isolation level: `set_isolation` on the manual handle.
//! - Nested transactions are rejected.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType, Default, PartialEq)]
#[table("txn_items")]
struct TxnItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

async fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();
    ctx.set::<TxnItem>();
    ctx.ensure_created().await.unwrap();
    ctx
}

async fn count_items(ctx: &mut DbContext) -> i64 {
    ctx.set::<TxnItem>().query().count().await.expect("count")
}

async fn add_item(ctx: &mut DbContext, name: &str) {
    ctx.add::<TxnItem>(TxnItem {
        id: 0,
        name: name.into(),
    });
}

#[tokio::test]
async fn test_begin_transaction_returns_handle() {
    let mut ctx = make_ctx().await;
    let txn = ctx.begin_transaction().await.expect("begin");
    add_item(&mut ctx, "a").await;
    ctx.save_changes().await.expect("save");
    txn.commit().await.expect("commit");
    assert_eq!(count_items(&mut ctx).await, 1);
}

#[tokio::test]
async fn test_begin_transaction_rollback_discards_writes() {
    let mut ctx = make_ctx().await;
    let txn = ctx.begin_transaction().await.expect("begin");
    add_item(&mut ctx, "a").await;
    ctx.save_changes().await.expect("save");
    txn.rollback().await.expect("rollback");
    ctx.clear_tracking();
    assert_eq!(count_items(&mut ctx).await, 0);
}

// -----------------------------------------------------------------------
// use_transaction: commit / rollback
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_use_transaction_commits_on_ok() {
    let mut ctx = make_ctx().await;

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            add_item(ctx, "alpha").await;
            ctx.save_changes().await?;
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

    assert_eq!(count_items(&mut ctx).await, 1);
}

#[tokio::test]
async fn test_use_transaction_rolls_back_on_err() {
    let mut ctx = make_ctx().await;

    let result: EFResult<()> = ctx
        .use_transaction(|ctx| {
            Box::pin(async move {
                add_item(ctx, "beta").await;
                ctx.save_changes().await?;
                Err(EFError::transaction("forced failure"))
            })
        })
        .await;

    assert!(result.is_err());
    assert_eq!(count_items(&mut ctx).await, 0);
}

#[tokio::test]
async fn test_use_transaction_multiple_save_changes_share_transaction() {
    let mut ctx = make_ctx().await;

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            add_item(ctx, "first").await;
            ctx.save_changes().await?;
            add_item(ctx, "second").await;
            ctx.save_changes().await?;
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

    assert_eq!(count_items(&mut ctx).await, 2);
}

// -----------------------------------------------------------------------
// Savepoints on a manual transaction handle
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_savepoint_create_and_rollback_to() {
    let mut ctx = make_ctx().await;
    let mut txn = ctx.begin_transaction().await.expect("begin");

    add_item(&mut ctx, "before_sp").await;
    ctx.save_changes().await.expect("save before");

    txn.create_point("sp1").await.expect("create point");
    add_item(&mut ctx, "after_sp").await;
    ctx.save_changes().await.expect("save after");
    txn.rollback_point("sp1").await.expect("rollback to point");
    txn.commit().await.expect("commit");

    let items = ctx.set::<TxnItem>().query().to_list().await.expect("list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "before_sp");
}

#[tokio::test]
async fn test_savepoint_release_then_rollback_to_fails() {
    let mut ctx = make_ctx().await;
    let mut txn = ctx.begin_transaction().await.expect("begin");

    add_item(&mut ctx, "x").await;
    ctx.save_changes().await.expect("save");

    txn.create_point("sp1").await.expect("create point");
    txn.release_point("sp1").await.expect("release point");
    let result = txn.rollback_point("sp1").await;
    assert!(result.is_err(), "rollback after release should fail");

    txn.rollback().await.expect("rollback");
}

#[tokio::test]
async fn test_set_isolation_level_serializable() {
    let mut ctx = make_ctx().await;

    let mut txn = ctx.begin_transaction().await.expect("begin");
    txn.set_isolation(IsolationLevel::Serializable)
        .await
        .expect("set isolation");
    add_item(&mut ctx, "iso").await;
    ctx.save_changes().await.expect("save");
    txn.commit().await.expect("commit");

    assert_eq!(count_items(&mut ctx).await, 1);
}

// -----------------------------------------------------------------------
// Error cases
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_nested_use_transaction_errors() {
    let mut ctx = make_ctx().await;

    let result: EFResult<()> = ctx
        .use_transaction(|ctx| {
            Box::pin(async move {
                ctx.use_transaction(|_ctx| Box::pin(async move { Ok(()) }))
                    .await?;
                Ok(())
            })
        })
        .await;

    assert!(result.is_err(), "nested transaction should fail");
}
