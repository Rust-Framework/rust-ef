//! Shared transaction-scope integration fixture for MySQL/PostgreSQL tests.

#![allow(dead_code)]

use rust_ef::error::EFResult;
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::prelude::*;
use rust_ef::provider::IDatabaseProvider;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, EntityType, Default)]
#[table("txn_test_items")]
pub struct TxnTestItem {
    #[primary_key]
    #[auto_increment]
    #[required]
    pub id: i32,
    #[required]
    pub name: String,
}

pub async fn run_transaction_scope(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    let engine = MigrationEngine::new(dialect);
    let meta = TxnTestItem::entity_meta();
    let _ = engine
        .ensure_deleted(&*provider, std::slice::from_ref(&meta))
        .await;
    engine
        .ensure_created(&*provider, std::slice::from_ref(&meta))
        .await?;

    let cleanup_provider = Arc::clone(&provider);
    let mut ctx = super::db_context_with_provider(provider);
    ctx.set::<TxnTestItem>();

    // Manual rollback: save_changes joins, nothing is committed.
    let txn = ctx.begin_transaction().await?;
    ctx.add::<TxnTestItem>(TxnTestItem {
        id: 0,
        name: "rollback".into(),
    });
    ctx.save_changes().await?;
    assert_eq!(ctx.set::<TxnTestItem>().query().count().await?, 1);
    txn.rollback().await?;
    ctx.clear_tracking();
    assert_eq!(ctx.set::<TxnTestItem>().query().count().await?, 0);

    // Manual commit.
    let txn = ctx.begin_transaction().await?;
    ctx.add::<TxnTestItem>(TxnTestItem {
        id: 0,
        name: "commit".into(),
    });
    ctx.save_changes().await?;
    txn.commit().await?;
    assert_eq!(ctx.set::<TxnTestItem>().query().count().await?, 1);

    // Isolation at transaction start.
    let txn = ctx
        .begin_transaction_with(IsolationLevel::ReadCommitted)
        .await?;
    let mut item = ctx.set::<TxnTestItem>().query().to_list().await?.remove(0);
    item.name = "changed".into();
    ctx.update::<TxnTestItem>(item);
    ctx.save_changes().await?;
    txn.commit().await?;

    let rows = ctx.set::<TxnTestItem>().query().to_list().await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "changed");

    // Dropping an unfinished transaction must close/discard the connection so
    // the database rolls back; the context must remain usable afterwards.
    let txn = ctx.begin_transaction().await?;
    ctx.add::<TxnTestItem>(TxnTestItem {
        id: 0,
        name: "dropped".into(),
    });
    ctx.save_changes().await?;
    drop(txn);

    let mut count = -1;
    for _ in 0..60 {
        count = ctx.set::<TxnTestItem>().query().count().await?;
        if count == 1 {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(count, 1, "dropped transaction must roll back");

    let _ = engine
        .ensure_deleted(&*cleanup_provider, std::slice::from_ref(&meta))
        .await;
    Ok(())
}
