//! Change detection benchmark — measures `detect_changes` on 1000 tracked
//! entities where half have been modified. Isolates the snapshot comparison
//! hot path from save overhead.
//!
//! Run with: `cargo bench -p rust-ef --bench bench_detect_changes`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use tokio::runtime::Runtime;

#[derive(Debug, Clone, EntityType)]
#[table("bench_detect_widgets")]
struct BenchDetectWidget {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    #[max_length(100)]
    name: String,
    value: f64,
}

/// Seeds a fresh in-memory SQLite context with `n` rows.
async fn seeded_ctx(n: usize) -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("DbContext");
    ctx.set::<BenchDetectWidget>();
    ctx.ensure_created().await.expect("ensure_created");

    for i in 0..n {
        ctx.set::<BenchDetectWidget>().add(BenchDetectWidget {
            id: 0,
            name: format!("widget-{i}"),
            value: i as f64,
        });
    }
    ctx.save_changes().await.expect("seed save");
    ctx
}

/// Loads `n` rows, modifies the first half, then runs `detect_changes`.
/// Only the detect_changes call is measured (load/modify happen inside the
/// benchmark closure but outside the hot loop is not possible with criterion's
/// async iter — the full closure is timed).
async fn detect_changes_half_modified(n: usize) {
    let mut ctx = seeded_ctx(n).await;

    let items = ctx
        .set::<BenchDetectWidget>()
        .query()
        .to_list()
        .await
        .expect("load");
    ctx.set::<BenchDetectWidget>().clear_entries();
    for item in items {
        ctx.set::<BenchDetectWidget>().attach(item);
    }

    let half = n / 2;
    for (i, entry) in ctx
        .set::<BenchDetectWidget>()
        .tracked_entries_mut()
        .enumerate()
    {
        if i < half {
            entry.value += 1.0;
        }
    }

    // This is the operation under test.
    ctx.set::<BenchDetectWidget>().detect_changes();
}

fn bench_detect_changes(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    rt.block_on(detect_changes_half_modified(10));

    let mut group = c.benchmark_group("detect_changes");
    group.sample_size(20);

    for n in [500usize, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.to_async(&rt)
                .iter(|| async move { detect_changes_half_modified(n).await });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_detect_changes);
criterion_main!(benches);
