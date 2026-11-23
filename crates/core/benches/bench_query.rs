//! Batch SELECT benchmark — measures `to_list` query throughput for tables
//! pre-seeded with 100 / 500 / 1000 rows, both unfiltered and filtered.
//!
//! Run with: `cargo bench -p rust-ef --bench bench_query`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::{AggKind, CompareOp, HavingExpr, SelectItem};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

/// Simple single-PK entity used for query throughput measurement.
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

/// Seeds a fresh in-memory SQLite context with `n` rows.
async fn seeded_ctx(n: usize) -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("DbContext");
    ctx.set::<BenchWidget>();
    ctx.ensure_created().await.expect("ensure_created");
    for i in 0..n {
        ctx.add::<BenchWidget>(BenchWidget {
            id: 0,
            name: format!("widget-{i}"),
            value: i as f64 * 1.5,
            tenant_id: (i / 10) as i32,
        });
    }
    let result = ctx.save_changes().await.expect("save_changes");
    assert_eq!(result.added, n, "seed should insert all rows");
    ctx
}

/// Unfiltered `to_list` — loads every row.
async fn select_all(ctx: &Mutex<DbContext>) {
    let mut guard = ctx.lock().await;
    let rows = guard
        .set::<BenchWidget>()
        .query()
        .to_list()
        .await
        .expect("to_list");
    assert!(!rows.is_empty(), "should load rows");
}

/// Filtered `to_list` via `linq!` — loads a subset matching a tenant.
async fn select_filtered(ctx: &Mutex<DbContext>) {
    let mut guard = ctx.lock().await;
    let rows = linq!(guard.set::<BenchWidget>(), |w: BenchWidget| w.tenant_id
        == 1)
    .to_list()
    .await
    .expect("to_list");
    assert!(!rows.is_empty(), "should load matching rows");
}

fn bench_batch_select(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");

    let mut group = c.benchmark_group("batch_select");

    // Unfiltered full-table scan. Seed once per size; the context is reused
    // across all samples of that size (queries are read-only).
    for &n in &[100usize, 500, 1000] {
        let ctx = Arc::new(Mutex::new(rt.block_on(seeded_ctx(n))));
        group.bench_with_input(BenchmarkId::new("to_list_all", n), &n, |b, _| {
            let ctx = ctx.clone();
            b.to_async(&rt).iter(move || {
                let ctx = ctx.clone();
                async move { select_all(&ctx).await }
            });
        });
    }

    // Filtered query (WHERE tenant_id = 1).
    for &n in &[100usize, 500, 1000] {
        let ctx = Arc::new(Mutex::new(rt.block_on(seeded_ctx(n))));
        group.bench_with_input(BenchmarkId::new("to_list_filtered", n), &n, |b, _| {
            let ctx = ctx.clone();
            b.to_async(&rt).iter(move || {
                let ctx = ctx.clone();
                async move { select_filtered(&ctx).await }
            });
        });
    }

    group.finish();
}

/// Measures the single-pass `compile_sql` (2.8) against the pre-2.8 pattern
/// of compiling SQL and parameters separately (`to_sql` + `all_params`).
fn bench_compile_pipeline(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");
    let mut ctx = rt.block_on(seeded_ctx(0));
    let having = HavingExpr::Compare {
        agg: AggKind::Sum,
        col: "value".to_string(),
        op: CompareOp::Gt,
        value: DbValue::F64(10.0),
    };
    let query = ctx
        .set::<BenchWidget>()
        .query()
        .filter(linq!(|w: BenchWidget| w.tenant_id == 1 && w.value > 1.0))
        .select_items(&[
            SelectItem::literal("TENANT", "scope"),
            SelectItem::column("tenant_id"),
            SelectItem::sum("value").coalesce(0i64),
        ])
        .group_by(&["tenant_id"])
        .having(having)
        .order_by("tenant_id")
        .take(10);

    let mut group = c.benchmark_group("compile_pipeline");
    group.bench_function("single_pass_compile_sql", |b| {
        b.iter(|| {
            let (sql, params) = query.compile_sql().expect("compile");
            assert!(sql.contains("GROUP BY") && params.len() >= 3);
        });
    });
    group.bench_function("two_pass_to_sql_all_params", |b| {
        b.iter(|| {
            let sql = query.to_sql();
            let params = query.state().all_params();
            assert!(sql.contains("GROUP BY") && params.len() >= 3);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_batch_select, bench_compile_pipeline);
criterion_main!(benches);
