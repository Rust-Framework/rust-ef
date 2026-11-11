//! Async `DbContext` registration (2.1.0): provider initialization during
//! `build_async`, typed failures, and unchanged synchronous owned resolution.

use rust_dix::RdiError;
use rust_ef::db_context::DbContext;
use rust_ef::di::{DbContextServiceCollectionExt, ServiceCollection, ServiceProvider};
use rust_ef::error::{EFError, EFResult};
use rust_ef::prelude::*;
use rust_ef::provider::IDatabaseProvider;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, EntityType)]
#[table("async_items")]
struct AsyncItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

async fn exercise(ctx: &mut DbContext, expected_count: i64) {
    ctx.set::<AsyncItem>();
    ctx.ensure_created().await.expect("ensure created");
    ctx.add::<AsyncItem>(AsyncItem {
        id: 0,
        name: "async".into(),
    });
    ctx.save_changes().await.expect("save");
    assert_eq!(
        ctx.set::<AsyncItem>().query().count().await.unwrap(),
        expected_count
    );
}

#[tokio::test]
async fn async_registration_initializes_during_build_async() {
    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_async(|o| {
            o.use_sqlite_in_memory();
        })
        .build_async()
        .await
        .expect("build_async");

    // The DbContext stays a synchronous Scoped service: handlers keep using
    // get_owned()/owned injection.
    let mut ctx: DbContext = provider.get_owned().expect("owned context");
    exercise(&mut ctx, 1).await;

    // `use_sqlite_in_memory()` shares one in-memory database per provider, so
    // the second context sees the first row.
    let scope = provider.scope();
    let mut scoped_ctx: DbContext = scope.get_owned().expect("scoped owned context");
    exercise(&mut scoped_ctx, 2).await;
}

#[tokio::test]
async fn keyed_async_registration_initializes_during_build_async() {
    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_keyed_async("primary", |o| {
            o.use_default_entity_model();
            o.use_sqlite_in_memory();
        })
        .build_async()
        .await
        .expect("build_async");
    let mut ctx: DbContext = provider
        .get_keyed_owned("primary")
        .expect("keyed owned context");
    exercise(&mut ctx, 1).await;
}

#[tokio::test]
async fn async_preflight_runs_after_provider_creation() {
    let called = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&called);

    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_async(move |options| {
            options.use_sqlite_in_memory();
            let flag = Arc::clone(&flag);
            options.async_preflight(move |options| {
                let flag = Arc::clone(&flag);
                Box::pin(async move {
                    // Provider/pool must already be initialized before preflight.
                    let provider = options.create_provider()?;
                    let mut conn = provider.get_connection().await?;
                    conn.query("SELECT 1", &[]).await?;
                    flag.store(true, Ordering::SeqCst);
                    Ok(())
                })
            });
        })
        .build_async()
        .await
        .expect("build_async with preflight");

    assert!(called.load(Ordering::SeqCst), "preflight must run");
    provider.dispose();
}

#[tokio::test]
async fn async_preflight_failure_aborts_build_async() {
    let result = ServiceCollection::new()
        .add_dbcontext_async(|options| {
            options.use_sqlite_in_memory();
            options.async_preflight(|_options| {
                Box::pin(async { Err(EFError::configuration("preflight failed")) })
            });
        })
        .build_async()
        .await;

    match result {
        Err(RdiError::FactoryPanic { payload, .. }) => {
            assert!(
                payload.contains("preflight failed"),
                "payload should carry the preflight error: {payload}"
            );
        }
        Err(other) => panic!("expected FactoryPanic, got {other:?}"),
        Ok(_) => panic!("build_async must fail when preflight fails"),
    }
}

#[test]
fn async_registration_requires_build_async() {
    let result = ServiceCollection::new()
        .add_dbcontext_async(|o| {
            o.use_sqlite_in_memory();
        })
        .build();
    match result {
        Err(RdiError::AsyncServiceRequiresBuildAsync { .. }) => {}
        Err(other) => panic!("expected AsyncServiceRequiresBuildAsync, got {other:?}"),
        Ok(_) => panic!("sync build must fail when async registrations are present"),
    }
}

#[tokio::test]
async fn provider_failure_is_a_typed_error_not_a_registration_panic() {
    type FailingFactory = dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync;
    let failing: Arc<FailingFactory> =
        Arc::new(|_| Err(EFError::configuration("intentional provider failure")));

    let result = ServiceCollection::new()
        .add_dbcontext_async(move |o| {
            o.set_provider_factory("failing", "unused", Arc::clone(&failing));
        })
        .build_async()
        .await;

    match result {
        Err(RdiError::FactoryPanic { .. }) => {}
        Err(other) => panic!("expected FactoryPanic, got {other:?}"),
        Ok(_) => panic!("provider failure must fail build_async"),
    }
}
