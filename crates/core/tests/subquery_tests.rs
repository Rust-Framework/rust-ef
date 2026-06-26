//! Tests for G5: correlated subqueries (`EXISTS` / `NOT EXISTS`).
//!
//! Verifies that `linq!` correctly expands `b.posts.any(|p: Post| p.published)`,
//! `b.posts.none(...)`, and `b.posts.all(...)` into correlated subquery SQL.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("sub_blogs")]
struct SubBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    url: String,
    #[navigation]
    posts: HasMany<SubPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("sub_posts")]
struct SubPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(SubBlog)]
    blog_id: i32,
    published: bool,
    #[navigation]
    blog: BelongsTo<SubBlog>,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Seeds 4 blogs with varying post publication states:
///
/// - Blog 1: posts = [published, unpublished] → any=T, all=F, none=F
/// - Blog 2: posts = [published, published]   → any=T, all=T, none=F
/// - Blog 3: posts = [unpublished]            → any=F, all=F, none=T
/// - Blog 4: no posts                         → any=F, all=T(vacuous), none=T
async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<SubBlog>();
    ctx.set::<SubPost>();
    ctx.ensure_created().await.unwrap();

    let blogs = [
        (
            "https://b1.example",
            vec![("Post 1A", true), ("Post 1B", false)],
        ),
        (
            "https://b2.example",
            vec![("rust intro", true), ("rust advanced", true)],
        ),
        ("https://b3.example", vec![("Post 3A", false)]),
        ("https://b4.example", vec![]),
    ];

    for (url, posts) in &blogs {
        ctx.set::<SubBlog>().add(SubBlog {
            blog_id: 0,
            url: (*url).into(),
            posts: HasMany::new(),
        });
        ctx.save_changes().await.unwrap();

        let inserted = ctx.set::<SubBlog>().query().to_list().await.unwrap();
        let blog_id = inserted.last().unwrap().blog_id;

        for (title, published) in posts {
            ctx.set::<SubPost>().add(SubPost {
                post_id: 0,
                title: (*title).into(),
                blog_id,
                published: *published,
                blog: BelongsTo::new(),
            });
        }
        ctx.save_changes().await.unwrap();
    }

    ctx
}

#[tokio::test]
async fn test_any_published_post() {
    // `b.posts.any(|p: SubPost| p.published)` → blogs with ≥1 published post.
    // Expected: Blog 1, Blog 2.
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b
        .posts
        .any(|p: SubPost| p.published))
    .to_list()
    .await
    .unwrap();
    let urls: Vec<String> = blogs.iter().map(|b| b.url.clone()).collect();
    assert_eq!(blogs.len(), 2, "expected 2 blogs with published posts");
    assert!(urls.contains(&"https://b1.example".to_string()));
    assert!(urls.contains(&"https://b2.example".to_string()));
}

#[tokio::test]
async fn test_none_published_post() {
    // `b.posts.none(|p: SubPost| p.published)` → blogs with 0 published posts.
    // Expected: Blog 3, Blog 4.
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b
        .posts
        .none(|p: SubPost| p.published))
    .to_list()
    .await
    .unwrap();
    let urls: Vec<String> = blogs.iter().map(|b| b.url.clone()).collect();
    assert_eq!(blogs.len(), 2, "expected 2 blogs with no published posts");
    assert!(urls.contains(&"https://b3.example".to_string()));
    assert!(urls.contains(&"https://b4.example".to_string()));
}

#[tokio::test]
async fn test_all_posts_published() {
    // `b.posts.all(|p: SubPost| p.published)` → blogs where every post is published.
    // Expected: Blog 2 (all published), Blog 4 (vacuously true — no posts).
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b
        .posts
        .all(|p: SubPost| p.published))
    .to_list()
    .await
    .unwrap();
    let urls: Vec<String> = blogs.iter().map(|b| b.url.clone()).collect();
    assert_eq!(blogs.len(), 2, "expected 2 blogs where all posts published");
    assert!(urls.contains(&"https://b2.example".to_string()));
    assert!(urls.contains(&"https://b4.example".to_string()));
}

#[tokio::test]
async fn test_not_any_published() {
    // `!b.posts.any(|p: SubPost| p.published)` ≡ `none(...)` → Blog 3, Blog 4.
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| !b
        .posts
        .any(|p: SubPost| p.published))
    .to_list()
    .await
    .unwrap();
    assert_eq!(
        blogs.len(),
        2,
        "expected 2 blogs with no published posts (via !any)"
    );
}

#[tokio::test]
async fn test_any_with_complex_predicate() {
    // `b.posts.any(|p: SubPost| p.title.contains("rust"))` → Blog 2 only.
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b
        .posts
        .any(|p: SubPost| p.title.contains("rust")))
    .to_list()
    .await
    .unwrap();
    assert_eq!(blogs.len(), 1, "expected 1 blog with rust-titled posts");
    assert_eq!(blogs[0].url, "https://b2.example");
}

#[tokio::test]
async fn test_any_combined_with_and() {
    // `b.blog_id > 0 && b.posts.any(|p: SubPost| p.published)` → Blog 1, Blog 2.
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b.blog_id > 0
        && b.posts.any(|p: SubPost| p.published))
    .to_list()
    .await
    .unwrap();
    assert_eq!(
        blogs.len(),
        2,
        "expected 2 blogs matching combined condition"
    );
}

#[tokio::test]
async fn test_form_c_filter_subquery() {
    // Form C: `linq!(filter |b: SubBlog| b.posts.any(...))` produces a BoolExpr
    // value (used for `has_query_filter`). Verify the Exists variant is built.
    let expr: BoolExpr = linq!(filter |b: SubBlog| b.posts.any(|p: SubPost| p.published));
    assert!(
        matches!(expr, BoolExpr::Exists(_)),
        "Form C filter with .any() should produce BoolExpr::Exists"
    );

    // NOT EXISTS via `none(...)` in Form C.
    let expr_none: BoolExpr = linq!(filter |b: SubBlog| b.posts.none(|p: SubPost| p.published));
    assert!(
        matches!(expr_none, BoolExpr::NotExists(_)),
        "Form C filter with .none() should produce BoolExpr::NotExists"
    );
}

#[tokio::test]
async fn test_not_all_published() {
    // `!b.posts.all(|p: SubPost| p.published)` ≡ `any(|p| !p.published)`
    // → Blog 1 (has unpublished), Blog 3 (has unpublished).
    // Blog 2: all published → !all = false. Blog 4: vacuously all → !all = false.
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| !b
        .posts
        .all(|p: SubPost| p.published))
    .to_list()
    .await
    .unwrap();
    let urls: Vec<String> = blogs.iter().map(|b| b.url.clone()).collect();
    assert_eq!(
        blogs.len(),
        2,
        "expected 2 blogs where NOT all posts published"
    );
    assert!(urls.contains(&"https://b1.example".to_string()));
    assert!(urls.contains(&"https://b3.example".to_string()));
}
