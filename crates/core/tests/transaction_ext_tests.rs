//! Integration tests for the ITransaction abstraction (Priority 3, Part 0):
//! - `begin_transaction()` returns a typed `Box<dyn ITransaction>` handle.
//! - `use_transaction()` registers an ambient transaction so `save_changes()`
//!   reuses it; commits on `Ok`, rolls back on `Err`.
//! - Savepoints: `create_point` / `release_point` / `rollback_point` on the
//!   ambient handle accessed via `transaction_mut()`.
//! - Isolation level: `set_isolation` on the ambient handle.
//! - Error cases: `transaction_mut()` returns `None` without ambient,
//!   nested `use_transaction` is rejected.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

// -----------------------------------------------------------------------
// Entity
// -----------------------------------------------------------------------

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
    ctx.set::<TxnItem>().add(TxnItem {
        id: 0,
        name: name.into(),
    });
}

// -----------------------------------------------------------------------
// begin_transaction returns a handle (manual control, no ambient)
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_begin_transaction_returns_handle() {
    let mut ctx = make_ctx().await;
    let txn = ctx.begin_transaction().await.expect("begin");
    // Handle is not registered as ambient — transaction_mut() returns None.
    assert!(
        ctx.transaction_mut().is_none(),
        "begin_transaction should not register ambient"
    );
    txn.commit().await.expect("commit");
}

#[tokio::test]
async fn test_begin_transaction_rollback_consumes_handle() {
    let mut ctx = make_ctx().await;
    let txn = ctx.begin_transaction().await.expect("begin");
    txn.rollback().await.expect("rollback");
    // Handle consumed; no ambient to check.
    assert!(ctx.transaction_mut().is_none());
}

// -----------------------------------------------------------------------
// use_transaction: ambient + save_changes integration
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

    assert_eq!(
        count_items(&mut ctx).await,
        1,
        "item should persist after use_transaction commit"
    );
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

    assert!(result.is_err(), "use_transaction should propagate error");
    assert_eq!(
        count_items(&mut ctx).await,
        0,
        "save_changes writes should be rolled back"
    );
}

#[tokio::test]
async fn test_use_transaction_multiple_save_changes_share_ambient() {
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

    assert_eq!(
        count_items(&mut ctx).await,
        2,
        "both save_changes calls should share the ambient transaction"
    );
}

// -----------------------------------------------------------------------
// Savepoints via transaction_mut() inside use_transaction
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_savepoint_create_and_rollback_to() {
    let mut ctx = make_ctx().await;

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            add_item(ctx, "before_sp").await;
            ctx.save_changes().await?;

            {
                let txn = ctx.transaction_mut().expect("ambient");
                txn.create_point("sp1").await?;
            }

            add_item(ctx, "after_sp").await;
            ctx.save_changes().await?;

            {
                let txn = ctx.transaction_mut().expect("ambient");
                txn.rollback_point("sp1").await?;
            }
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

    let items = ctx.set::<TxnItem>().query().to_list().await.expect("list");
    assert_eq!(items.len(), 1, "only pre-savepoint item should remain");
    assert_eq!(items[0].name, "before_sp");
}

#[tokio::test]
async fn test_savepoint_release_then_rollback_to_fails() {
    let mut ctx = make_ctx().await;

    let result: EFResult<()> = ctx
        .use_transaction(|ctx| {
            Box::pin(async move {
                add_item(ctx, "x").await;
                ctx.save_changes().await?;

                {
                    let txn = ctx.transaction_mut().expect("ambient");
                    txn.create_point("sp1").await?;
                    txn.release_point("sp1").await?;
                }
                {
                    let txn = ctx.transaction_mut().expect("ambient");
                    // After release, rollback_to should fail at the DB level.
                    txn.rollback_point("sp1").await?;
                }
                Ok(())
            })
        })
        .await;

    assert!(
        result.is_err(),
        "rollback_point after release_point should fail"
    );
}

#[tokio::test]
async fn test_transaction_mut_none_without_ambient() {
    let mut ctx = make_ctx().await;
    assert!(
        ctx.transaction_mut().is_none(),
        "transaction_mut should return None without use_transaction"
    );
}

// -----------------------------------------------------------------------
// Isolation level via transaction_mut()
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_set_isolation_level_serializable() {
    let mut ctx = make_ctx().await;

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            {
                let txn = ctx.transaction_mut().expect("ambient");
                txn.set_isolation(IsolationLevel::Serializable).await?;
            }
            add_item(ctx, "iso").await;
            ctx.save_changes().await?;
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

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
                // Nested use_transaction should fail because ambient is active.
                ctx.use_transaction(|_ctx| Box::pin(async move { Ok(()) }))
                    .await?;
                Ok(())
            })
        })
        .await;

    assert!(
        result.is_err(),
        "nested use_transaction should return an error"
    );
}
