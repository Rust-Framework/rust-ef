//! Include (eager load) vs N+1 benchmark — compares loading `n_blogs` blogs
//! each with `posts_per_blog` posts via:
//!   1. A single `linq!(...; include b.posts)` query (1 + 1 round trips).
//!   2. An N+1 pattern: load blogs, then one query per blog for its posts
//!      (1 + N round trips).
//!
//! Run with: `cargo bench -p rust-ef --bench bench_include`

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::Mutex;

#[derive(Debug, Clone, EntityType)]
#[table("bench_blogs")]
struct BenchBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    #[max_length(200)]
    url: String,
    #[navigation]
    posts: HasMany<BenchPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("bench_posts")]
struct BenchPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    #[max_length(200)]
    title: String,
    #[foreign_key(BenchBlog)]
    blog_id: i32,
}

/// Seeds a fresh in-memory SQLite context with `n_blogs` blogs, each carrying
/// `posts_per_blog` posts.
async fn seeded_ctx(n_blogs: usize, posts_per_blog: usize) -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("DbContext");
    ctx.set::<BenchBlog>();
    ctx.set::<BenchPost>();
    ctx.ensure_created().await.expect("ensure_created");

    for b in 0..n_blogs {
        ctx.add::<BenchBlog>(BenchBlog {
            blog_id: 0,
            url: format!("https://blog-{b}.example"),
            posts: HasMany::new(),
        });
    }
    let saved = ctx.save_changes().await.expect("save_changes blogs");
    assert_eq!(saved.added, n_blogs);

    let blogs = ctx
        .set::<BenchBlog>()
        .query()
        .to_list()
        .await
        .expect("blogs");
    for blog in &blogs {
        for p in 0..posts_per_blog {
            ctx.add::<BenchPost>(BenchPost {
                post_id: 0,
                title: format!("post-{}-{}", blog.blog_id, p),
                blog_id: blog.blog_id,
            });
        }
    }
    let saved_posts = ctx.save_changes().await.expect("save_changes posts");
    assert_eq!(saved_posts.added, n_blogs * posts_per_blog);
    ctx
}

/// Eager load: a single `linq!(...; include b.posts)` query fetches all blogs
/// and their posts in 2 round trips (blogs + posts-by-blog-id).
async fn include_load(ctx: &Mutex<DbContext>) {
    let mut guard = ctx.lock().await;
    let blogs = linq!(guard.set::<BenchBlog>(); include b.posts)
        .to_list()
        .await
        .expect("include to_list");
    assert!(!blogs.is_empty(), "blogs loaded");
    for b in &blogs {
        assert!(!b.posts.is_empty(), "posts eager-loaded for each blog");
    }
}

/// N+1 load: load all blogs, then one query per blog for its posts.
async fn n_plus_one_load(ctx: &Mutex<DbContext>) {
    let mut guard = ctx.lock().await;
    let blogs = guard
        .set::<BenchBlog>()
        .query()
        .to_list()
        .await
        .expect("blogs");
    assert!(!blogs.is_empty(), "blogs loaded");
    let mut total_posts = 0usize;
    for blog in &blogs {
        let posts = linq!(guard.set::<BenchPost>(), |p: BenchPost| p.blog_id
            == blog.blog_id)
        .to_list()
        .await
        .expect("posts per blog");
        total_posts += posts.len();
    }
    assert!(total_posts > 0, "posts loaded via N+1");
}

fn bench_include_vs_n_plus_one(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio runtime");

    // 50 blogs × 10 posts = 500 child rows. Large enough to make the N+1
    // round-trip overhead visible, small enough to keep bench wall time sane.
    let n_blogs = 50usize;
    let posts_per_blog = 10usize;

    let mut group = c.benchmark_group("include_vs_n_plus_one");
    group.sample_size(20);

    {
        let ctx = Arc::new(Mutex::new(rt.block_on(seeded_ctx(n_blogs, posts_per_blog))));
        group.bench_with_input(
            BenchmarkId::new("include", format!("{n_blogs}x{posts_per_blog}")),
            &(),
            |b, _| {
                let ctx = ctx.clone();
                b.to_async(&rt).iter(move || {
                    let ctx = ctx.clone();
                    async move { include_load(&ctx).await }
                });
            },
        );
    }

    {
        let ctx = Arc::new(Mutex::new(rt.block_on(seeded_ctx(n_blogs, posts_per_blog))));
        group.bench_with_input(
            BenchmarkId::new("n_plus_one", format!("{n_blogs}x{posts_per_blog}")),
            &(),
            |b, _| {
                let ctx = ctx.clone();
                b.to_async(&rt).iter(move || {
                    let ctx = ctx.clone();
                    async move { n_plus_one_load(&ctx).await }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_include_vs_n_plus_one);
criterion_main!(benches);
