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

// -----------------------------------------------------------------------
// use_transaction_generic: a non-EFError application error boundary
// -----------------------------------------------------------------------

/// Application error type: implements `From<EFError>` once, exactly like a
/// webx error boundary would, so shared use cases can stay generic.
#[derive(Debug)]
enum DomainError {
    Ef(EFError),
    Business(&'static str),
}

impl From<EFError> for DomainError {
    fn from(error: EFError) -> Self {
        Self::Ef(error)
    }
}

#[tokio::test]
async fn test_use_transaction_generic_commits_on_ok() {
    let mut ctx = make_ctx().await;

    let result: Result<u8, DomainError> = ctx
        .use_transaction_generic(|ctx| {
            Box::pin(async move {
                add_item(ctx, "generic-ok").await;
                ctx.save_changes().await?; // EFError -> DomainError via From
                Ok(7u8)
            })
        })
        .await;

    assert_eq!(result.expect("generic transaction"), 7);
    assert_eq!(count_items(&mut ctx).await, 1);
}

#[tokio::test]
async fn test_use_transaction_generic_rolls_back_and_preserves_error() {
    let mut ctx = make_ctx().await;

    let result: Result<(), DomainError> = ctx
        .use_transaction_generic(|ctx| {
            Box::pin(async move {
                add_item(ctx, "generic-rollback").await;
                ctx.save_changes().await?;
                Err(DomainError::Business("insufficient balance"))
            })
        })
        .await;

    match result {
        Err(DomainError::Business(reason)) => assert_eq!(reason, "insufficient balance"),
        other => panic!("expected the original business error, got {other:?}"),
    }
    assert_eq!(count_items(&mut ctx).await, 0, "Err must roll back");
}

#[tokio::test]
async fn test_use_transaction_generic_converts_framework_errors() {
    let mut ctx = make_ctx().await;

    let result: Result<(), DomainError> = ctx
        .use_transaction_generic(|_ctx| {
            Box::pin(async move { Err(EFError::transaction("framework failure").into()) })
        })
        .await;

    match result {
        Err(DomainError::Ef(EFError::Transaction(reason, _))) => {
            assert_eq!(reason, "framework failure");
        }
        other => panic!("EFError must be converted through From, got {other:?}"),
    }
}

// -----------------------------------------------------------------------
// use_transaction_map_err: downstream error types that cannot impl From<EFError>
// -----------------------------------------------------------------------

/// Mimics a Web-layer error type owned by another crate: Rust's orphan rule
/// lets neither the application nor the framework add
/// `impl From<EFError> for BoundaryError`, so the mapping is passed explicitly.
#[derive(Debug)]
enum BoundaryError {
    Mapped(String),
}

#[tokio::test]
async fn test_use_transaction_map_err_commits_on_ok() {
    let mut ctx = make_ctx().await;

    let result: Result<u8, BoundaryError> = ctx
        .use_transaction_map_err(
            |_| unreachable!("mapper must not run when the closure returns Ok"),
            |ctx| {
                Box::pin(async move {
                    add_item(ctx, "map-ok").await;
                    // 2.x recipe: framework calls inside the closure are mapped
                    // by the caller; only transaction-boundary errors use the
                    // `map_err` argument.
                    ctx.save_changes()
                        .await
                        .map_err(|error| BoundaryError::Mapped(error.to_string()))?;
                    Ok(7u8)
                })
            },
        )
        .await;

    assert_eq!(result.expect("map_err transaction"), 7);
    assert_eq!(count_items(&mut ctx).await, 1);
}

#[tokio::test]
async fn test_use_transaction_map_err_rolls_back_and_returns_closure_error_verbatim() {
    let mut ctx = make_ctx().await;

    let result: Result<(), BoundaryError> = ctx
        .use_transaction_map_err(
            // If the mapper were applied to the closure's error, this panic
            // would surface instead of the original error.
            |error| panic!("mapper must not run for the closure error: {error}"),
            |ctx| {
                Box::pin(async move {
                    add_item(ctx, "map-rollback").await;
                    ctx.save_changes().await.expect("setup write");
                    Err(BoundaryError::Mapped("business rejection".into()))
                })
            },
        )
        .await;

    match result {
        Err(BoundaryError::Mapped(reason)) => assert_eq!(reason, "business rejection"),
        other => panic!("closure error must be returned verbatim, got {other:?}"),
    }
    assert_eq!(count_items(&mut ctx).await, 0, "Err must roll back");
}

#[tokio::test]
async fn test_use_transaction_map_err_maps_transaction_start_errors() {
    let mut ctx = make_ctx().await;

    // Open a manual transaction: the nested closure must fail at transaction
    // start, and that framework error goes through `map_err`.
    let txn = ctx.begin_transaction().await.expect("begin");
    let nested: Result<(), BoundaryError> = ctx
        .use_transaction_map_err(
            |error| BoundaryError::Mapped(error.to_string()),
            |_ctx| Box::pin(async move { Ok(()) }),
        )
        .await;
    txn.rollback().await.expect("rollback");

    match nested {
        Err(BoundaryError::Mapped(message)) => {
            assert!(
                message.contains("already active"),
                "mapper must receive the transaction-start error, got: {message}"
            );
        }
        other => panic!("expected a mapped transaction-start error, got {other:?}"),
    }
    assert_eq!(count_items(&mut ctx).await, 0);
}

// -----------------------------------------------------------------------
// §10: bulk DML joins the active transaction implicitly
// -----------------------------------------------------------------------

async fn first_item_name(ctx: &mut DbContext) -> String {
    ctx.set::<TxnItem>()
        .query()
        .to_list()
        .await
        .expect("list")
        .into_iter()
        .next()
        .expect("one row")
        .name
}

/// `execute_update` goes through the context session provider, so inside
/// `use_transaction` it runs on the transaction connection and commits with it.
#[tokio::test]
async fn test_execute_update_commits_with_use_transaction() {
    let mut ctx = make_ctx().await;
    add_item(&mut ctx, "before").await;
    ctx.save_changes().await.expect("seed");

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            let affected = ctx
                .set::<TxnItem>()
                .query()
                .filter(linq!(|i: TxnItem| i.name == "before"))
                .execute_update()
                .set_column(TxnItem::COLUMN_NAME, "after")
                .execute()
                .await?;
            assert_eq!(affected, 1);
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

    assert_eq!(first_item_name(&mut ctx).await, "after");
}

/// On `Err` the transaction (and the bulk DML issued inside it) rolls back; the
/// in-transaction query still sees the uncommitted write, proving the DML used
/// the transaction connection instead of a separate pooled one.
#[tokio::test]
async fn test_execute_update_rolls_back_with_use_transaction() {
    let mut ctx = make_ctx().await;
    add_item(&mut ctx, "before").await;
    ctx.save_changes().await.expect("seed");

    let result: EFResult<()> = ctx
        .use_transaction(|ctx| {
            Box::pin(async move {
                let affected = ctx
                    .set::<TxnItem>()
                    .query()
                    .filter(linq!(|i: TxnItem| i.name == "before"))
                    .execute_update()
                    .set_column(TxnItem::COLUMN_NAME, "after")
                    .execute()
                    .await?;
                assert_eq!(affected, 1);

                let inside = ctx
                    .set::<TxnItem>()
                    .query()
                    .filter(linq!(|i: TxnItem| i.name == "after"))
                    .first_or_default()
                    .await?
                    .expect("uncommitted DML is visible inside the transaction");
                assert_eq!(inside.name, "after");

                Err(EFError::transaction("abort"))
            })
        })
        .await;

    assert!(
        result.is_err(),
        "use_transaction must surface the closure error"
    );
    assert_eq!(
        first_item_name(&mut ctx).await,
        "before",
        "rollback must discard the bulk DML"
    );
}
