//! Integration tests for v1.1 `IN (SELECT ...)` / `NOT IN (SELECT ...)`
//! subquery support via `b.field.in_subquery(|p: Post| p.blog_id)`.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("in_sub_blogs")]
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
#[table("in_sub_posts")]
struct SubPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(SubBlog)]
    blog_id: i32,
    #[navigation]
    blog: BelongsTo<SubBlog>,
}

fn build_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("DbContext")
}

async fn seed(ctx: &mut DbContext) {
    ctx.set::<SubBlog>();
    ctx.set::<SubPost>();
    ctx.ensure_created().await.unwrap();

    // Two blogs.
    ctx.add::<SubBlog>(SubBlog {
        blog_id: 0,
        url: "https://a.example".into(),
        posts: HasMany::new(),
    });
    ctx.add::<SubBlog>(SubBlog {
        blog_id: 0,
        url: "https://b.example".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await.unwrap();

    let blogs = ctx.set::<SubBlog>().query().to_list().await.unwrap();
    let blog_a = blogs[0].blog_id;

    // Blog A has 2 posts; Blog B has 0 posts.
    ctx.add::<SubPost>(SubPost {
        post_id: 0,
        title: "A1".into(),
        blog_id: blog_a,
        blog: BelongsTo::new(),
    });
    ctx.add::<SubPost>(SubPost {
        post_id: 0,
        title: "A2".into(),
        blog_id: blog_a,
        blog: BelongsTo::new(),
    });
    ctx.save_changes().await.unwrap();
}

// ---------------------------------------------------------------------------
// Form A/B (method chain) tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_in_subquery_form_a() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // SELECT * FROM in_sub_blogs WHERE blog_id IN (SELECT blog_id FROM in_sub_posts)
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b
        .blog_id
        .in_subquery(|p: SubPost| p.blog_id))
    .to_list()
    .await
    .unwrap();

    // Only Blog A has posts → only Blog A should match.
    assert_eq!(blogs.len(), 1);
    assert_eq!(blogs[0].url, "https://a.example");
}

#[tokio::test]
async fn test_not_in_subquery_form_a() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // SELECT * FROM in_sub_blogs WHERE blog_id NOT IN (SELECT blog_id FROM in_sub_posts)
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| !b
        .blog_id
        .in_subquery(|p: SubPost| p.blog_id))
    .to_list()
    .await
    .unwrap();

    // Blog B has no posts → should match NOT IN.
    assert_eq!(blogs.len(), 1);
    assert_eq!(blogs[0].url, "https://b.example");
}

#[tokio::test]
async fn test_in_subquery_combined_with_other_filter() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // WHERE url = 'https://a.example' AND blog_id IN (SELECT blog_id FROM posts)
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b.url
        == "https://a.example"
        && b.blog_id.in_subquery(|p: SubPost| p.blog_id))
    .to_list()
    .await
    .unwrap();

    assert_eq!(blogs.len(), 1);
    assert_eq!(blogs[0].url, "https://a.example");

    // Combining with a non-matching url → empty.
    let blogs2 = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b.url
        == "https://b.example"
        && b.blog_id.in_subquery(|p: SubPost| p.blog_id))
    .to_list()
    .await
    .unwrap();
    assert!(blogs2.is_empty());
}

// ---------------------------------------------------------------------------
// Form C (global query filter) tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_in_subquery_form_c_filter() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // Apply a global filter using Form C syntax.
    ctx.set::<SubBlog>()
        .set_query_filter(linq!(filter |b: SubBlog| b.blog_id.in_subquery(|p: SubPost| p.blog_id)));

    let blogs = ctx.set::<SubBlog>().query().to_list().await.unwrap();

    // Only Blog A has posts.
    assert_eq!(blogs.len(), 1);
    assert_eq!(blogs[0].url, "https://a.example");
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_in_subquery_empty_subquery_result() {
    let mut ctx = build_ctx();
    ctx.set::<SubBlog>();
    ctx.set::<SubPost>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<SubBlog>(SubBlog {
        blog_id: 0,
        url: "https://empty.example".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await.unwrap();

    // No posts exist → IN (SELECT blog_id FROM posts) returns empty set → no blogs match.
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| b
        .blog_id
        .in_subquery(|p: SubPost| p.blog_id))
    .to_list()
    .await
    .unwrap();

    assert!(blogs.is_empty());
}

#[tokio::test]
async fn test_not_in_subquery_empty_subquery_result() {
    let mut ctx = build_ctx();
    ctx.set::<SubBlog>();
    ctx.set::<SubPost>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<SubBlog>(SubBlog {
        blog_id: 0,
        url: "https://empty.example".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await.unwrap();

    // No posts exist → NOT IN (SELECT blog_id FROM posts) → all blogs match.
    let blogs = linq!(ctx.set::<SubBlog>(), |b: SubBlog| !b
        .blog_id
        .in_subquery(|p: SubPost| p.blog_id))
    .to_list()
    .await
    .unwrap();

    assert_eq!(blogs.len(), 1);
    assert_eq!(blogs[0].url, "https://empty.example");
}
