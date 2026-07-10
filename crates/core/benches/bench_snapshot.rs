//! Snapshot allocation benchmark — measures `entity.snapshot()` and
//! `entity.key_values()` allocation overhead directly (no DB I/O).
//!
//! Isolates the `EntitySnapshot` construction cost from save/detect overhead.
//! The macro-generated `snapshot()` builds a `Box<[(&'static str, DbValue)]>`
//! in a single heap allocation; `key_values()` builds a smaller snapshot with
//! only PK columns.
//!
//! Run with: `cargo bench -p rust-ef --bench bench_snapshot`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rust_ef::entity::{IEntitySnapshot, IGetKeyValues};
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("bench_snap_small")]
struct SmallEntity {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    #[max_length(100)]
    name: String,
    value: f64,
    active: bool,
    count: i64,
}

#[derive(Debug, Clone, EntityType)]
#[table("bench_snap_medium")]
struct MediumEntity {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    #[max_length(100)]
    name: String,
    value: f64,
    active: bool,
    count: i64,
    score: f32,
    tag: String,
    priority: i32,
    weight: f64,
    flags: i64,
}

fn make_small(i: usize) -> SmallEntity {
    SmallEntity {
        id: i as i32,
        name: format!("small-{i}"),
        value: i as f64,
        active: i.is_multiple_of(2),
        count: i as i64,
    }
}

fn make_medium(i: usize) -> MediumEntity {
    MediumEntity {
        id: i as i32,
        name: format!("medium-{i}"),
        value: i as f64,
        active: i.is_multiple_of(2),
        count: i as i64,
        score: i as f32 * 0.5,
        tag: format!("tag-{i}"),
        priority: (i % 10) as i32,
        weight: i as f64 * 2.5,
        flags: (i * 3) as i64,
    }
}

fn bench_snapshot(c: &mut Criterion) {
    let small = make_small(0);
    let medium = make_medium(0);

    let mut group = c.benchmark_group("snapshot");

    group.bench_function("5_fields", |b| {
        b.iter(|| small.snapshot());
    });

    group.bench_function("10_fields", |b| {
        b.iter(|| medium.snapshot());
    });

    group.finish();

    let mut group = c.benchmark_group("key_values");

    group.bench_function("5_fields", |b| {
        b.iter(|| small.key_values());
    });

    group.bench_function("10_fields", |b| {
        b.iter(|| medium.key_values());
    });

    group.finish();
}

fn bench_snapshot_batch(c: &mut Criterion) {
    let batch: Vec<SmallEntity> = (0..1000).map(make_small).collect();

    let mut group = c.benchmark_group("snapshot_batch_1000");

    for &n in &[100usize, 500, 1000] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                for e in &batch[..n] {
                    e.snapshot();
                }
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_snapshot, bench_snapshot_batch);
criterion_main!(benches);
