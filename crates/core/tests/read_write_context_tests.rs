//! Read/write separation via two keyed `DbContext`s.
//!
//! This is the EF Core-style pattern: the application registers a write
//! context and a read context with different options, and injects both into a
//! handler. rust-ef performs no automatic routing; the handler chooses the
//! context explicitly.

use rust_dix::*;
use rust_ef::db_context::DbContext;
use rust_ef::di::DbContextServiceCollectionExt as _;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, EntityType, Default, PartialEq)]
#[table("rw_items")]
struct RwItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

fn db_path(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rust_ef_rw_{}_{}_{}.db",
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

#[inject(scoped)]
struct ReadWriteHandler {
    #[inject(owned, key = "write")]
    write: DbContext,
    #[inject(owned, key = "read")]
    read: DbContext,
}

impl ReadWriteHandler {
    async fn handle(&mut self) -> EFResult<(i64, i64)> {
        // The shared default entity model is discovered automatically by
        // `ensure_created`, without an explicit `set::<RwItem>()`.
        self.write.ensure_created().await?;

        self.write.add::<RwItem>(RwItem {
            id: 0,
            name: "created".into(),
        });
        self.write.save_changes().await?;

        // The write context sees its own committed write on the primary.
        let from_write = self.write.set::<RwItem>().query().count().await?;
        // The read context has its own pool; for SQLite it reads the same file.
        let from_read = self.read.set::<RwItem>().query().count().await?;
        Ok((from_write, from_read))
    }
}

#[tokio::test]
async fn keyed_read_write_contexts_use_separate_options_and_pools() {
    let path = db_path("contexts");
    let path_str = path.to_str().expect("utf-8 temp path").to_string();

    let write_cs = path_str.clone();
    let read_cs = path_str.clone();
    let provider: std::sync::Arc<ServiceProvider> = ServiceCollection::from_injected()
        .add_dbcontext_keyed("write", move |options| {
            options.use_default_entity_model();
            // Write context: a single pooled connection serializes writes.
            options.use_sqlite_with(&write_cs, |options| {
                options.max_pool_size(1);
            });
        })
        .add_dbcontext_keyed("read", move |options| {
            options.use_default_entity_model();
            // Read context: the default pool for concurrent reads.
            options.use_sqlite(&read_cs);
        })
        .build()
        .expect("build provider");

    let mut handler: ReadWriteHandler = provider.get_owned().expect("read/write handler");
    let (from_write, from_read) = handler.handle().await.expect("handler");

    assert_eq!(from_write, 1, "write context must see its own write");
    assert_eq!(from_read, 1, "read context must see the committed row");

    cleanup(&path);
}

#[tokio::test]
async fn keyed_replica_without_shared_model_has_actionable_error() {
    let path = db_path("missing_shared_model");
    let cs = path.to_str().expect("utf-8 temp path").to_string();
    let provider: std::sync::Arc<ServiceProvider> = ServiceCollection::new()
        .add_dbcontext_keyed("read", move |options| {
            // Deliberately omits use_default_entity_model().
            options.use_sqlite(&cs);
        })
        .build()
        .expect("build provider");

    let ctx: DbContext = provider.get_keyed_owned("read").expect("read context");
    let err = match ctx.ensure_created().await {
        Ok(()) => panic!("keyed context without the shared model must fail"),
        Err(err) => err,
    };
    assert!(
        err.message().contains("use_default_entity_model"),
        "error must point at the fix, got: {err}"
    );
    cleanup(&path);
}

#[test]
fn sqlite_pool_rejects_zero_and_memory() {
    let err = match rust_ef_sqlite::SqliteProvider::new_with_pool(":memory:", 1) {
        Ok(_) => panic!("in-memory pool must be rejected"),
        Err(err) => err,
    };
    assert!(matches!(err, EFError::Configuration(..)));

    let path = db_path("zero");
    let err = match rust_ef_sqlite::SqliteProvider::new_with_pool(&path, 0) {
        Ok(_) => panic!("zero-sized pool must be rejected"),
        Err(err) => err,
    };
    assert!(matches!(err, EFError::Configuration(..)));
    cleanup(&path);
}
