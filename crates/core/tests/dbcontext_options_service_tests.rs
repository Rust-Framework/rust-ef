//! Sharing the host's `Arc<DbContextOptions>` (upstream feedback §15).
//!
//! Durable framework components — a SQL-backed idempotency store, an outbox
//! dispatcher, background workers — create their own short-lived `DbContext`.
//! They must use the **same** options as the scoped `DbContext`, otherwise every
//! one of them opens a second connection pool and rebuilds the metadata cache.
//!
//! `add_dbcontext` / `add_dbcontext_with_options` publish the built options as a
//! `DbContextOptions` singleton so those consumers resolve them from DI instead
//! of receiving them out of band.

use rust_dix::IServiceResolverExt as _;
use rust_ef::db_context::{DbContext, DbContextOptions, DbContextOptionsBuilder};
use rust_ef::di::{DbContextServiceCollectionExt, ServiceCollection};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::Arc;

/// Stand-in for `EfIdempotencyStore` / `OutboxDispatcher`: a process-level
/// consumer that keeps the host options and opens its own short-lived context.
struct FakeDurableStore {
    options: Arc<DbContextOptions>,
}

#[test]
fn add_dbcontext_publishes_the_built_options() {
    let provider = ServiceCollection::new()
        .add_dbcontext(|options| {
            options.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider");

    let options: Arc<DbContextOptions> = provider
        .get()
        .expect("add_dbcontext must publish the built DbContextOptions");

    let scope = provider.scope();
    let ctx: Arc<DbContext> = scope.get().expect("scoped DbContext");
    let from_published = DbContext::from_options(&options).expect("context from published options");
    assert!(
        ctx.shares_metadata_with(&from_published),
        "the DI DbContext must be built from the published options (one metadata cache)"
    );

    // The provider (connection pool) is cached inside the options, so every
    // consumer resolving the same options reuses the same pool `Arc`.
    let pool = options.create_provider().expect("provider");
    let again = options.create_provider().expect("provider");
    assert!(
        Arc::ptr_eq(&pool, &again),
        "create_provider must return the cached pool"
    );
}

#[test]
fn add_dbcontext_with_options_shares_the_supplied_options() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = Arc::new(builder.build());

    let provider = ServiceCollection::new()
        .add_dbcontext_with_options(Arc::clone(&options))
        .build()
        .expect("build provider");

    let resolved: Arc<DbContextOptions> = provider
        .get()
        .expect("add_dbcontext_with_options must publish the supplied options");
    assert!(
        Arc::ptr_eq(&options, &resolved),
        "the host-supplied options Arc must be the one registered in DI"
    );

    let scope = provider.scope();
    let ctx: Arc<DbContext> = scope.get().expect("scoped DbContext");
    let from_options = DbContext::from_options(&options).expect("context from host options");
    assert!(
        ctx.shares_metadata_with(&from_options),
        "the scoped DbContext must use the host-supplied options"
    );
}

#[test]
fn process_level_consumer_and_scoped_context_share_one_pool() {
    let provider = ServiceCollection::new()
        .add_dbcontext(|options| {
            options.use_sqlite_in_memory();
        })
        .singleton(|resolver| {
            // §15 target shape: the store resolves the host options from DI.
            let options = resolver
                .resolve_arc::<DbContextOptions>()
                .expect("DbContextOptions");
            Arc::new(FakeDurableStore { options })
        })
        .build()
        .expect("build provider");

    let store: Arc<FakeDurableStore> = provider.get().expect("durable store");
    let resolved: Arc<DbContextOptions> = provider.get().expect("published options");
    assert!(
        Arc::ptr_eq(&store.options, &resolved),
        "the store and the scoped DbContext must resolve the same options Arc"
    );

    let scope = provider.scope();
    let ctx: Arc<DbContext> = scope.get().expect("scoped DbContext");
    let from_store = DbContext::from_options(&store.options).expect("context from store options");
    assert!(
        ctx.shares_metadata_with(&from_store),
        "the store and the scoped DbContext must share one metadata cache"
    );
    assert!(
        Arc::ptr_eq(
            &store.options.create_provider().expect("provider"),
            &store.options.create_provider().expect("provider")
        ),
        "the shared options reuse one connection pool"
    );
}
