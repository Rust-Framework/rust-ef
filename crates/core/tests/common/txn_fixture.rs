//! Shared transaction-scope integration fixture for MySQL/PostgreSQL tests.

#![allow(dead_code)]

use rust_ef::error::EFResult;
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::prelude::*;
use rust_ef::provider::IDatabaseProvider;
use std::sync::Arc;

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

    // Rollback with a business value.
    let outcome = ctx
        .transaction(|ctx| {
            Box::pin(async move {
                ctx.add::<TxnTestItem>(TxnTestItem {
                    id: 0,
                    name: "rollback".into(),
                });
                ctx.save_changes().await?;
                assert_eq!(ctx.set::<TxnTestItem>().query().count().await?, 1);
                Ok(TxOutcome::Rollback("rejected"))
            })
        })
        .await?;
    assert_eq!(outcome, "rejected");
    assert_eq!(ctx.set::<TxnTestItem>().query().count().await?, 0);

    // Commit multiple statements in the same unit of work.
    let outcome = ctx
        .transaction(|ctx| {
            Box::pin(async move {
                ctx.add::<TxnTestItem>(TxnTestItem {
                    id: 0,
                    name: "commit".into(),
                });
                ctx.save_changes().await?;
                Ok(TxOutcome::Commit("ok"))
            })
        })
        .await?;
    assert_eq!(outcome, "ok");
    assert_eq!(ctx.set::<TxnTestItem>().query().count().await?, 1);

    // Isolation at transaction start.
    let outcome = ctx
        .transaction_with(IsolationLevel::ReadCommitted, |ctx| {
            Box::pin(async move {
                let mut item = ctx.set::<TxnTestItem>().query().to_list().await?.remove(0);
                item.name = "changed".into();
                ctx.update::<TxnTestItem>(item);
                ctx.save_changes().await?;
                Ok(TxOutcome::Commit("done"))
            })
        })
        .await?;
    assert_eq!(outcome, "done");
    let rows = ctx.set::<TxnTestItem>().query().to_list().await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "changed");

    let _ = engine
        .ensure_deleted(&*cleanup_provider, std::slice::from_ref(&meta))
        .await;
    Ok(())
}
