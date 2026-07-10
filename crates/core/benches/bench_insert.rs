//! Batch INSERT benchmark — measures `save_changes` throughput for bulk inserts
//! of 100 / 500 / 1000 rows in a single transaction.
//!
//! Run with: `cargo bench -p rust-ef --bench bench_insert`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use tokio::runtime::Runtime;

/// Simple single-PK entity used for insert throughput measurement.
#[derive(Debug, Clone, EntityType)]
#[table("bench_widgets")]
struct BenchWidget {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    #[max_length(100)]
    name: String,
    value: f64,
    tenant_id: i32,
}

/// Builds a fresh in-memory SQLite context with the schema created.
async fn fresh_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("DbContext");
    ctx.set::<BenchWidget>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

/// Inserts `n` rows in a single `save_changes` call (one transaction, batched
/// into multi-value INSERT statements by the change executor).
async fn insert_n(n: usize) {
    let mut ctx = fresh_ctx().await;
    for i in 0..n {
        ctx.add::<BenchWidget>(BenchWidget {
            id: 0,
            name: format!("widget-{i}"),
            value: i as f64 * 1.5,
            tenant_id: (i / 10) as i32,
        });
    }
    let result = ctx.save_changes().await.expect("save_changes");
    assert_eq!(result.added, n, "all rows should be inserted");
}

fn bench_batch_insert(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    // Warm up / verify the helper works before measurement.
    rt.block_on(insert_n(10));

    let mut group = c.benchmark_group("batch_insert");
    for n in [100usize, 500, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.to_async(&rt).iter(|| async move { insert_n(n).await });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_batch_insert);
criterion_main!(benches);
