//! Dynamic (`AnyKey`) keyed `DbContext` registration (2.1.0).

use rust_dix::*;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::di::{DbContextServiceCollectionExt, ServiceProvider};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, EntityType)]
#[table("anykey_items")]
struct AnyItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

fn db_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rust_ef_anykey_{}_{}_{}.db",
        std::process::id(),
        tag,
        n
    ))
}

fn cleanup(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

#[tokio::test]
async fn anykey_builds_isolated_contexts_per_requested_key() {
    let dir = std::env::temp_dir();
    let base = format!("rust_ef_anykey_{}", std::process::id());
    // Windows can't delete the DB (or its `-wal`) while the pool still holds it,
    // so a previous run whose pid was recycled can leave rows behind. Start from
    // a clean file instead of trusting the previous run's cleanup.
    cleanup(&dir.join(format!("{}_{}.db", base, "tenant-a")));
    cleanup(&dir.join(format!("{}_{}.db", base, "tenant-b")));
    let configure_dir = dir.clone();
    let configure_base = base.clone();
    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_anykey(move |key, options: &mut DbContextOptionsBuilder| {
            let file = configure_dir.join(format!("{}_{}.db", configure_base, key));
            options.use_sqlite(file.to_string_lossy().as_ref());
        })
        .build()
        .expect("build provider");

    let scope = provider.scope();

    let mut a: DbContext = scope
        .get_keyed_owned("tenant-a")
        .expect("owned tenant-a context");
    a.set::<AnyItem>();
    a.ensure_created().await.expect("ensure created a");
    a.add::<AnyItem>(AnyItem {
        id: 0,
        name: "a".into(),
    });
    a.save_changes().await.expect("save a");
    assert_eq!(a.set::<AnyItem>().query().count().await.unwrap(), 1);

    // A different requested key gets its own database.
    let mut b: DbContext = scope
        .get_keyed_owned("tenant-b")
        .expect("owned tenant-b context");
    b.set::<AnyItem>();
    b.ensure_created().await.expect("ensure created b");
    assert_eq!(
        b.set::<AnyItem>().query().count().await.unwrap(),
        0,
        "dynamic keys must not share data"
    );

    // Shared resolution is cached per (registration, requested key) in the scope.
    let shared_a: Arc<DbContext> = scope.get_keyed("tenant-a").expect("shared a");
    let shared_a2: Arc<DbContext> = scope.get_keyed("tenant-a").expect("shared a2");
    assert!(Arc::ptr_eq(&shared_a, &shared_a2));

    cleanup(&dir.join(format!("{}_{}.db", base, "tenant-a")));
    cleanup(&dir.join(format!("{}_{}.db", base, "tenant-b")));
}

#[tokio::test]
async fn anykey_options_cache_is_bounded_lru() {
    let builds = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&builds);
    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_anykey_with_capacity(2, move |_key, options| {
            counter.fetch_add(1, Ordering::SeqCst);
            options.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider");

    let resolve = |key: &str| -> DbContext {
        provider
            .get_keyed_owned(key)
            .unwrap_or_else(|err| panic!("resolve {key}: {err:?}"))
    };

    let _ = resolve("a");
    let _ = resolve("b");
    assert_eq!(builds.load(Ordering::SeqCst), 2);

    // Capacity 2: "a" is still cached, so no rebuild.
    let _ = resolve("a");
    assert_eq!(builds.load(Ordering::SeqCst), 2);

    // "c" evicts the least-recently-used key ("b").
    let _ = resolve("c");
    assert_eq!(builds.load(Ordering::SeqCst), 3);

    // "b" was evicted and is rebuilt.
    let _ = resolve("b");
    assert_eq!(builds.load(Ordering::SeqCst), 4);
}

#[test]
fn exact_key_registration_takes_precedence_over_anykey() {
    let calls = Arc::new(AtomicUsize::new(0));
    let anykey_calls = Arc::clone(&calls);

    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_keyed("primary", |o| {
            o.use_sqlite_in_memory();
        })
        .add_dbcontext_anykey(move |_key, o| {
            anykey_calls.fetch_add(1, Ordering::SeqCst);
            o.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider");

    let _primary: DbContext = provider
        .get_keyed_owned("primary")
        .expect("exact registration");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the AnyKey factory must not run for an exact key"
    );

    let _dynamic: DbContext = provider
        .get_keyed_owned("dynamic")
        .expect("anykey fallback");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn anykey_context_uses_the_default_entity_model() {
    let path = db_path("model");
    cleanup(&path);
    let cs = path.to_string_lossy().to_string();
    let provider: Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_anykey(move |_key, o| {
            o.use_sqlite(&cs);
        })
        .build()
        .expect("build provider");

    let scope = provider.scope();
    let ctx: DbContext = scope.get_keyed_owned("tenant-model").expect("ctx");
    assert!(
        ctx.entity_metas_contains::<AnyItem>(),
        "AnyKey contexts must use the shared default entity model"
    );
    ctx.ensure_created().await.expect("ensure created");
    cleanup(&path);
}
