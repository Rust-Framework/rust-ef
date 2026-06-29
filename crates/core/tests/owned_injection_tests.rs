//! End-to-end owned injection tests.
//!
//! Verifies the complete flow:
//!   add_dbcontext (scoped) → #[inject] auto-registers + auto-detects bare
//!   DbContext field → get_owned::<Handler>() → handle(&mut self) →
//!   self.ctx.set::<T>()
//!
//! This proves the framework is ready for the EFCore-aligned handler pattern
//! without Arc<Mutex> or interior mutability.

use rust_dicore::{inject, ServiceCollection};
use rust_ef::db_context::DbContext;
use rust_ef::di::DbContextServiceCollectionExt as _;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::Arc;

// ── Test entity ──

#[derive(Debug, Clone, EntityType)]
#[table("widgets")]
struct Widget {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

// ── Handler with owned DbContext (bare T field) ──
//
// #[inject] on the struct does two things:
//   1. Generates a constructor that resolves fields via DI
//   2. Registers the type via inventory for ServiceCollection::from_injected()
//
// The bare `ctx: DbContext` field is auto-classified as Owned → resolved via
// get_owned(), giving the handler exclusive &mut self access.

#[inject(scoped)]
struct CreateWidgetHandler {
    ctx: DbContext, // bare T → owned resolution via get_owned()
}

impl CreateWidgetHandler {
    async fn handle(&mut self, name: &str) -> EFResult<()> {
        // In-memory SQLite: each owned DbContext gets a fresh database,
        // so ensure_created() must be called before DML operations.
        self.ctx.set::<Widget>();
        self.ctx.ensure_created().await?;

        let widget = Widget {
            id: 0,
            name: name.to_string(),
        };
        self.ctx.set::<Widget>().add(widget);
        self.ctx.save_changes().await?;
        Ok(())
    }
}

// ── Handler with Arc<DbContext> (shared resolution, &self only) ──

#[inject(scoped)]
struct SharedContextHandler {
    ctx: Arc<DbContext>, // Arc<T> → shared resolution via get()
}

impl SharedContextHandler {
    // Arc<DbContext> only provides &self access — can call &self methods
    // like ensure_created(), but NOT set::<T>() or save_changes() (&mut self).
    async fn ensure_schema(&self) -> EFResult<()> {
        self.ctx.ensure_created().await
    }
}

fn build_provider() -> Arc<rust_dicore::ServiceProvider> {
    Arc::new(
        ServiceCollection::from_injected()
            .add_dbcontext(|o| {
                o.use_sqlite_in_memory();
            })
            .build()
            .expect("build provider"),
    )
}

#[tokio::test]
async fn owned_handler_can_mutate_dbcontext() {
    // End-to-end: #[inject(scoped)] + bare DbContext field + &mut self methods.
    // Each get_owned() returns a fresh DbContext (fresh in-memory SQLite),
    // so the handler calls ensure_created() itself before DML.
    let provider = build_provider();

    // Resolve handler via get_owned — #[inject] auto-detects bare T
    // and calls get_owned() for the DbContext field.
    let mut handler: CreateWidgetHandler = provider.get_owned();
    let result = handler.handle("test-widget").await;
    assert!(
        result.is_ok(),
        "owned handler must be able to mutate DbContext"
    );
}

#[tokio::test]
async fn owned_handler_gets_fresh_instance_each_call() {
    // Each get_owned::<Handler>() creates a fresh Handler with a fresh DbContext.
    let provider = build_provider();

    let h1: CreateWidgetHandler = provider.get_owned();
    let h2: CreateWidgetHandler = provider.get_owned();

    let addr1 = &h1.ctx as *const _ as usize;
    let addr2 = &h2.ctx as *const _ as usize;
    assert_ne!(
        addr1, addr2,
        "each get_owned::<Handler>() must have a distinct DbContext"
    );
}

#[tokio::test]
async fn shared_handler_uses_arc_resolution() {
    // Arc<DbContext> field → #[inject] uses get() (shared Arc).
    // Arc<DbContext> only supports &self methods (e.g. ensure_created).
    let provider = build_provider();

    // Need to register the DbSet first via an owned context
    {
        let mut ctx: DbContext = provider.get_owned();
        ctx.set::<Widget>();
    }

    let handler: SharedContextHandler = provider.get_owned();
    let result = handler.ensure_schema().await;
    assert!(
        result.is_ok(),
        "shared handler must work with &self methods"
    );
}

#[tokio::test]
async fn keyed_owned_resolution_works() {
    // Verify keyed owned resolution: get_keyed_owned::<DbContext>("key").
    let provider = Arc::new(
        ServiceCollection::new()
            .add_dbcontext_keyed("primary", |o| {
                o.use_sqlite_in_memory();
            })
            .build()
            .unwrap(),
    );

    let mut ctx1: DbContext = provider.get_keyed_owned("primary");
    let mut ctx2: DbContext = provider.get_keyed_owned("primary");

    let addr1 = &ctx1 as *const _ as usize;
    let addr2 = &ctx2 as *const _ as usize;
    assert_ne!(addr1, addr2, "keyed owned must return fresh instances");

    // Verify the keyed context is usable
    ctx1.set::<Widget>();
    ctx1.ensure_created().await.unwrap();
    ctx2.set::<Widget>();
    ctx2.ensure_created().await.unwrap();
}
