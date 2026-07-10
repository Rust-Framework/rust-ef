//! Batch UPDATE and DELETE benchmark — measures `save_changes` throughput for
//! bulk updates (1000 rows via CASE WHEN batch) and bulk deletes (1000 rows
//! via IN clause) in a single transaction.
//!
//! Run with: `cargo bench -p rust-ef --bench bench_save`

use criterion::{criterion_group, criterion_main, Criterion};
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use tokio::runtime::Runtime;

#[derive(Debug, Clone, EntityType)]
#[table("bench_save_widgets")]
struct BenchSaveWidget {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    #[max_length(100)]
    name: String,
    value: f64,
}

/// Seeds a fresh in-memory SQLite context with `n` rows and returns it.
async fn seeded_ctx(n: usize) -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("DbContext");
    ctx.set::<BenchSaveWidget>();
    ctx.ensure_created().await.expect("ensure_created");

    for i in 0..n {
        ctx.set::<BenchSaveWidget>().add(BenchSaveWidget {
            id: 0,
            name: format!("widget-{i}"),
            value: i as f64,
        });
    }
    ctx.save_changes().await.expect("seed save");
    ctx
}

/// Loads `n` rows, modifies every entity, runs `detect_changes`, then saves.
/// The save executes a batched `UPDATE ... SET value = CASE id WHEN ? THEN ?
/// ... END WHERE id IN (...)`.
async fn batch_update(n: usize) {
    let mut ctx = seeded_ctx(n).await;

    // Load all rows and attach as Unchanged (with original snapshot).
    let items = ctx
        .set::<BenchSaveWidget>()
        .query()
        .to_list()
        .await
        .expect("load");
    ctx.set::<BenchSaveWidget>().clear_entries();
    for item in items {
        ctx.set::<BenchSaveWidget>().attach(item);
    }

    // Modify every entity.
    for entry in ctx.set::<BenchSaveWidget>().tracked_entries_mut() {
        entry.value += 1.0;
    }

    ctx.set::<BenchSaveWidget>().detect_changes();
    let result = ctx.save_changes().await.expect("save");
    assert_eq!(result.updated, n, "all rows should be updated");
}

/// Loads `n` rows, marks all as Deleted, then saves. The save executes a
/// batched `DELETE ... WHERE id IN (?, ?, ...)`.
async fn batch_delete(n: usize) {
    let mut ctx = seeded_ctx(n).await;

    let items = ctx
        .set::<BenchSaveWidget>()
        .query()
        .to_list()
        .await
        .expect("load");
    ctx.set::<BenchSaveWidget>().clear_entries();
    for item in items {
        ctx.set::<BenchSaveWidget>().attach(item);
    }

    ctx.set::<BenchSaveWidget>().remove_all();
    let result = ctx.save_changes().await.expect("save");
    assert_eq!(result.deleted, n, "all rows should be deleted");
}

fn bench_save(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    // Warm up / verify helpers work before measurement.
    rt.block_on(batch_update(10));
    rt.block_on(batch_delete(10));

    let n = 1000usize;
    let mut group = c.benchmark_group("batch_save");
    group.sample_size(20);

    group.bench_function("update_1000", |b| {
        b.to_async(&rt)
            .iter(|| async move { batch_update(n).await });
    });
    group.bench_function("delete_1000", |b| {
        b.to_async(&rt)
            .iter(|| async move { batch_delete(n).await });
    });

    group.finish();
}

criterion_group!(benches, bench_save);
criterion_main!(benches);
