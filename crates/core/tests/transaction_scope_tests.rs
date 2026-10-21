//! Regression tests for transaction scope.
//!
//! These tests use a **file-based** SQLite database (pooled connections). The
//! existing transaction tests use `:memory:`, where every connection handle
//! shares one underlying `rusqlite::Connection`; that masks the bug: inside
//! `use_transaction`, queries and bulk DML acquired a *different* pooled
//! connection, so they could not see uncommitted writes and bulk statements
//! committed immediately outside the transaction.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::error::{EFError, EFResult};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, EntityType, Default, PartialEq)]
#[table("txn_scope_items")]
struct TxnScopeItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
    value: i32,
}

#[derive(Debug, Clone, EntityType, Default, PartialEq)]
#[table("txn_scope_others")]
struct TxnScopeOther {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[unique]
    code: String,
}

fn db_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rust_ef_txn_scope_{}_{}_{}.db",
        std::process::id(),
        tag,
        n
    ))
}

fn cleanup(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{}", path.display(), suffix));
        let _ = std::fs::remove_file(file);
    }
}

async fn make_ctx(path: &Path) -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite(path.to_str().expect("utf-8 temp path"));
    let mut ctx = DbContext::from_options(&builder.build()).expect("DbContext");
    ctx.set::<TxnScopeItem>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

async fn seed(ctx: &mut DbContext) {
    ctx.add::<TxnScopeItem>(TxnScopeItem {
        id: 0,
        name: "a".into(),
        value: 1,
    });
    ctx.save_changes().await.expect("seed save_changes");
}

async fn value_of_a(ctx: &mut DbContext) -> i32 {
    ctx.set::<TxnScopeItem>()
        .query()
        .to_list()
        .await
        .expect("query")
        .into_iter()
        .find(|item| item.name == "a")
        .expect("row a")
        .value
}

#[tokio::test]
async fn txn_query_sees_uncommitted_orm_update() {
    let path = db_path("uncommitted");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            let mut item = ctx.set::<TxnScopeItem>().query().to_list().await?.remove(0);
            item.value = 2;
            ctx.update::<TxnScopeItem>(item);
            ctx.save_changes().await?;

            let seen = ctx.set::<TxnScopeItem>().query().to_list().await?;
            assert_eq!(
                seen[0].value, 2,
                "a query inside the transaction must see the uncommitted ORM update"
            );
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

    assert_eq!(value_of_a(&mut ctx).await, 2);
    cleanup(&path);
}

#[tokio::test]
async fn txn_rolls_back_bulk_execute_update() {
    let path = db_path("bulk_update");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let result: EFResult<()> = ctx
        .use_transaction(|ctx| {
            Box::pin(async move {
                let affected = linq!(ctx.set::<TxnScopeItem>(), |i: TxnScopeItem| i.name == "a";
                    set i.value, 9;
                    execute_update
                )
                .await?;
                assert_eq!(affected, 1);
                Err(EFError::transaction("abort"))
            })
        })
        .await;

    assert!(result.is_err());
    assert_eq!(
        value_of_a(&mut ctx).await,
        1,
        "execute_update must join the ambient transaction and roll back"
    );
    cleanup(&path);
}

#[tokio::test]
async fn txn_rolls_back_bulk_execute_delete() {
    let path = db_path("bulk_delete");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let result: EFResult<()> = ctx
        .use_transaction(|ctx| {
            Box::pin(async move {
                let affected = linq!(ctx.set::<TxnScopeItem>(), |i: TxnScopeItem| i.name == "a")
                    .execute_delete()
                    .await?;
                assert_eq!(affected, 1);
                Err(EFError::transaction("abort"))
            })
        })
        .await;

    assert!(result.is_err());
    assert_eq!(
        ctx.set::<TxnScopeItem>().query().count().await.unwrap(),
        1,
        "execute_delete must join the ambient transaction and roll back"
    );
    cleanup(&path);
}

#[tokio::test]
async fn txn_sql_query_sees_uncommitted_write() {
    let path = db_path("sql_query");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            let mut item = ctx.set::<TxnScopeItem>().query().to_list().await?.remove(0);
            item.value = 2;
            ctx.update::<TxnScopeItem>(item);
            ctx.save_changes().await?;

            let rows: Vec<TxnScopeItem> =
                ctx.sql_query("SELECT * FROM txn_scope_items", &[]).await?;
            assert_eq!(
                rows[0].value, 2,
                "sql_query inside the transaction must use the transaction connection"
            );
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

    cleanup(&path);
}

#[tokio::test]
async fn txn_outcome_rollback_returns_value_and_discards_writes() {
    let path = db_path("outcome_rollback");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let outcome = ctx
        .transaction(|ctx| {
            Box::pin(async move {
                linq!(ctx.set::<TxnScopeItem>(), |i: TxnScopeItem| i.name == "a";
                    set i.value, 9;
                    execute_update
                )
                .await?;

                let seen = ctx.set::<TxnScopeItem>().query().to_list().await?;
                assert_eq!(
                    seen.iter().find(|i| i.name == "a").unwrap().value,
                    9,
                    "the join query must see the uncommitted bulk update"
                );

                ctx.add::<TxnScopeItem>(TxnScopeItem {
                    id: 0,
                    name: "b".into(),
                    value: 5,
                });
                ctx.save_changes().await?;

                Ok(TxOutcome::Rollback("rejected"))
            })
        })
        .await
        .expect("transaction");

    assert_eq!(outcome, "rejected");
    assert_eq!(
        value_of_a(&mut ctx).await,
        1,
        "bulk update must be rolled back"
    );
    assert_eq!(
        ctx.set::<TxnScopeItem>().query().count().await.unwrap(),
        1,
        "ORM insert must be rolled back"
    );
    assert!(
        !ctx.change_tracker().has_changes(),
        "rollback must detach tracked entities"
    );
    cleanup(&path);
}

#[tokio::test]
async fn txn_outcome_commit_persists_all_writes() {
    let path = db_path("outcome_commit");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let committed = ctx
        .transaction(|ctx| {
            Box::pin(async move {
                ctx.add::<TxnScopeItem>(TxnScopeItem {
                    id: 0,
                    name: "b".into(),
                    value: 5,
                });
                ctx.save_changes().await?;

                linq!(ctx.set::<TxnScopeItem>(), |i: TxnScopeItem| i.name == "a";
                    set i.value, 9;
                    execute_update
                )
                .await?;

                let seen = ctx.set::<TxnScopeItem>().query().to_list().await?;
                assert_eq!(
                    seen.iter().find(|i| i.name == "a").unwrap().value,
                    9,
                    "the join query must see the uncommitted bulk update"
                );

                Ok(TxOutcome::Commit(2))
            })
        })
        .await
        .expect("transaction");

    assert_eq!(committed, 2);
    assert_eq!(value_of_a(&mut ctx).await, 9);
    assert_eq!(ctx.set::<TxnScopeItem>().query().count().await.unwrap(), 2);
    cleanup(&path);
}

#[tokio::test]
async fn txn_outcome_error_rolls_back_and_propagates() {
    let path = db_path("outcome_error");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let result: EFResult<()> = ctx
        .transaction(|ctx| {
            Box::pin(async move {
                linq!(ctx.set::<TxnScopeItem>(), |i: TxnScopeItem| i.name == "a";
                    set i.value, 9;
                    execute_update
                )
                .await?;
                Err(EFError::transaction("boom"))
            })
        })
        .await;

    assert!(result.is_err());
    assert_eq!(
        value_of_a(&mut ctx).await,
        1,
        "Err from the closure must roll the transaction back"
    );
    cleanup(&path);
}

#[tokio::test]
async fn failed_save_rolls_back_to_savepoint_and_keeps_transaction_usable() {
    let path = db_path("savepoint");
    let mut ctx = make_ctx(&path).await;
    ctx.set::<TxnScopeOther>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx.provider()
        .execute_migration_command(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_txn_scope_others_code \
             ON txn_scope_others (code)",
        )
        .await
        .expect("unique index");

    seed(&mut ctx).await;
    ctx.add::<TxnScopeOther>(TxnScopeOther {
        id: 0,
        code: "x".into(),
    });
    ctx.add::<TxnScopeOther>(TxnScopeOther {
        id: 0,
        code: "y".into(),
    });
    ctx.save_changes().await.expect("seed others");

    ctx.use_transaction(|ctx| {
        Box::pin(async move {
            // This INSERT succeeds before the duplicate UPDATE fails. The
            // savepoint must roll it back, or it would be committed with the
            // outer transaction.
            ctx.add::<TxnScopeItem>(TxnScopeItem {
                id: 0,
                name: "leak".into(),
                value: 5,
            });

            let mut other = ctx
                .set::<TxnScopeOther>()
                .query()
                .to_list()
                .await?
                .into_iter()
                .find(|o| o.code == "y")
                .expect("row y");
            other.code = "x".into();
            ctx.update::<TxnScopeOther>(other);

            let failed = ctx.save_changes().await;
            assert!(failed.is_err(), "duplicate code must fail the save");

            let leaked = ctx.set::<TxnScopeItem>().query().to_list().await?;
            assert!(
                !leaked.iter().any(|i| i.name == "leak"),
                "a failed save must be rolled back to its savepoint"
            );

            // The ambient transaction is still usable after the failed save.
            ctx.clear_tracking();
            ctx.add::<TxnScopeItem>(TxnScopeItem {
                id: 0,
                name: "after".into(),
                value: 6,
            });
            ctx.save_changes().await?;
            Ok(())
        })
    })
    .await
    .expect("use_transaction");

    let items = ctx.set::<TxnScopeItem>().query().to_list().await.unwrap();
    assert_eq!(items.len(), 2, "only the seed and 'after' should remain");
    cleanup(&path);
}

#[tokio::test]
async fn begin_transaction_with_serializable_isolation() {
    let path = db_path("isolation_begin");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let txn = ctx
        .begin_transaction_with(IsolationLevel::Serializable)
        .await
        .expect("begin_transaction_with");

    ctx.add::<TxnScopeItem>(TxnScopeItem {
        id: 0,
        name: "iso".into(),
        value: 3,
    });
    ctx.save_changes().await.expect("save");
    txn.commit().await.expect("commit");

    assert_eq!(ctx.set::<TxnScopeItem>().query().count().await.unwrap(), 2);
    cleanup(&path);
}

#[tokio::test]
async fn transaction_with_serializable_isolation_returns_value() {
    let path = db_path("isolation_transaction");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let outcome = ctx
        .transaction_with(IsolationLevel::Serializable, |ctx| {
            Box::pin(async move {
                let mut item = ctx.set::<TxnScopeItem>().query().to_list().await?.remove(0);
                item.value = 9;
                ctx.update::<TxnScopeItem>(item);
                ctx.save_changes().await?;
                Ok(TxOutcome::Rollback("rejected"))
            })
        })
        .await
        .expect("transaction_with");

    assert_eq!(outcome, "rejected");
    assert_eq!(value_of_a(&mut ctx).await, 1);
    cleanup(&path);
}

#[tokio::test]
async fn sqlite_rejects_unsupported_isolation_without_leaking_transaction() {
    let path = db_path("isolation_reject");
    let mut ctx = make_ctx(&path).await;
    seed(&mut ctx).await;

    let err = match ctx
        .begin_transaction_with(IsolationLevel::ReadCommitted)
        .await
    {
        Ok(_) => panic!("SQLite must reject non-serializable isolation"),
        Err(err) => err,
    };
    assert!(
        matches!(err, EFError::Provider(..)),
        "expected Provider error, got {err:?}"
    );

    // The failed begin must not leave a half-open transaction behind.
    let txn = ctx
        .begin_transaction()
        .await
        .expect("no transaction should remain registered");
    txn.rollback().await.expect("rollback");
    cleanup(&path);
}
