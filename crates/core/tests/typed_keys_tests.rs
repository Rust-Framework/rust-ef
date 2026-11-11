//! Typed keyed `DbContext` registration/resolution (2.1.0).

use rust_dix::*;
use rust_ef::db_context::DbContext;
use rust_ef::di::{
    DbContextResolver, DbContextServiceCollectionExt, IDbContextKey, ServiceKey, ServiceProvider,
};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, DbContextKey)]
enum DbKey {
    Read,
    Write,
    #[key("audit-log")]
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, DbContextKey)]
enum CustomKey {
    #[key("custom-context")]
    Custom,
}

#[inject(scoped)]
struct TypedWriteHandler {
    #[inject(owned, key = DbKey::Write)]
    ctx: DbContext,
}

#[derive(Debug, Clone, EntityType)]
#[context("read")]
#[table("typed_read_items")]
struct ReadItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

#[derive(Debug, Clone, EntityType)]
#[context("write")]
#[table("typed_write_items")]
struct WriteItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

fn build_provider() -> Arc<ServiceProvider> {
    ServiceCollection::new()
        .add_dbcontext_keyed(DbKey::Read, |o| {
            o.use_sqlite_in_memory();
        })
        .add_dbcontext_keyed(DbKey::Write, |o| {
            o.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider")
}

#[tokio::test]
async fn typed_keys_register_resolve_and_scope_entity_models() {
    let provider = build_provider();

    // Owned resolution from the root (allowed for Scoped contexts).
    let mut write = provider.get_owned_context(DbKey::Write).expect("write ctx");
    assert!(
        write.entity_metas_contains::<WriteItem>(),
        "typed key name must select the write context entity model"
    );
    assert!(
        !write.entity_metas_contains::<ReadItem>(),
        "write context must not discover read-tagged entities"
    );
    write.set::<WriteItem>();
    write.ensure_created().await.expect("ensure created");
    write.add::<WriteItem>(WriteItem {
        id: 0,
        name: "typed".into(),
    });
    write.save_changes().await.expect("save");
    assert_eq!(write.set::<WriteItem>().query().count().await.unwrap(), 1);

    // Shared resolution requires a scope under strict scope validation.
    let scope = provider.scope();
    let read: Arc<DbContext> = scope.get_context(DbKey::Read).expect("read ctx");
    assert!(read.entity_metas_contains::<ReadItem>());
    read.ensure_created().await.expect("ensure created");
}

#[test]
fn string_keys_stay_source_compatible() {
    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_keyed("legacy", |o| {
            o.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider");
    let _ctx: DbContext = provider
        .get_keyed_owned("legacy")
        .expect("string-keyed context");
    let _ctx: DbContext = provider
        .get_owned_context("legacy")
        .expect("typed-helper with string key");
}

#[test]
fn derived_key_feeds_typed_service_key_and_name() {
    assert_eq!(DbKey::Read.name(), "read");
    assert_eq!(DbKey::Write.name(), "write");
    assert_eq!(DbKey::Audit.name(), "audit-log");
    assert_eq!(CustomKey::Custom.name(), "custom-context");

    let key: ServiceKey = DbKey::Read.into();
    assert_eq!(key, ServiceKey::new(DbKey::Read));
    // Typed enum keys are not string keys; string registration stays a String key.
    assert_ne!(key, ServiceKey::from("read"));
}

#[tokio::test]
async fn typed_key_expression_injection_selects_the_keyed_context() {
    let provider: Arc<ServiceProvider> = ServiceCollection::from_injected()
        .add_dbcontext_keyed(DbKey::Read, |o| {
            o.use_sqlite_in_memory();
        })
        .add_dbcontext_keyed(DbKey::Write, |o| {
            o.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider");

    let mut handler: TypedWriteHandler = provider.get_owned().expect("typed handler");
    assert!(
        handler.ctx.entity_metas_contains::<WriteItem>(),
        "typed key expression must select the write context model"
    );
    assert!(!handler.ctx.entity_metas_contains::<ReadItem>());

    handler.ctx.set::<WriteItem>();
    handler.ctx.ensure_created().await.expect("ensure created");
    handler.ctx.add::<WriteItem>(WriteItem {
        id: 0,
        name: "typed-injection".into(),
    });
    handler.ctx.save_changes().await.expect("save");
    assert_eq!(
        handler
            .ctx
            .set::<WriteItem>()
            .query()
            .count()
            .await
            .unwrap(),
        1
    );
}
