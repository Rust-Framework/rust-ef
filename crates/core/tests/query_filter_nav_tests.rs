//! Verifies that query filters are applied to navigation property loading
//! (Include), preventing cross-tenant data from leaking into navigation
//! collections.
//!
//! Scenario: `NavTenantBlog` has many `NavTenantPost`. `NavTenantPost`
//! carries a `tenant_id` column and a query filter `tenant_id = 1`. When
//! loading blogs with `include b.posts`, only same-tenant posts should
//! appear in the navigation collection.
//!
//! Nested includes (`include b.posts then b.comments`) are also tested to
//! verify that `filter_map` is threaded through `load_nested_includes`.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::{BoolExpr, FilterCondition};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("nav_tenant_blogs")]
struct NavTenantBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    url: String,
    #[navigation]
    posts: HasMany<NavTenantPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("nav_tenant_posts")]
struct NavTenantPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(NavTenantBlog)]
    blog_id: i32,
    #[required]
    tenant_id: i32,
    #[navigation]
    comments: HasMany<NavTenantComment>,
}

#[derive(Debug, Clone, EntityType)]
#[table("nav_tenant_comments")]
struct NavTenantComment {
    #[primary_key]
    #[auto_increment]
    comment_id: i32,
    #[required]
    text: String,
    #[foreign_key(NavTenantPost)]
    post_id: i32,
    #[required]
    tenant_id: i32,
}

fn tenant_filter(value: i32) -> BoolExpr {
    BoolExpr::Filter(FilterCondition::with_values(
        "tenant_id",
        "=",
        vec![DbValue::I32(value)],
    ))
}

/// Builds a context with query filters `tenant_id = 1` on both
/// `NavTenantPost` and `NavTenantComment`.
async fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.model()
        .has_query_filter::<NavTenantPost>(tenant_filter(1))
        .has_query_filter::<NavTenantComment>(tenant_filter(1));
    ctx.set::<NavTenantBlog>();
    ctx.set::<NavTenantPost>();
    ctx.set::<NavTenantComment>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

#[tokio::test]
async fn hasmany_navigation_respects_tenant_filter() {
    let mut ctx = make_ctx().await;

    ctx.add::<NavTenantBlog>(NavTenantBlog {
        blog_id: 0,
        url: "https://tenant.example".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await.expect("insert blog");
    let blog_id = ctx
        .set::<NavTenantBlog>()
        .query()
        .to_list()
        .await
        .expect("query blog")[0]
        .blog_id;

    // Insert posts across two tenants. INSERTs are not filtered.
    ctx.add::<NavTenantPost>(NavTenantPost {
        post_id: 0,
        title: "own-tenant".into(),
        blog_id,
        tenant_id: 1,
        comments: HasMany::new(),
    });
    ctx.add::<NavTenantPost>(NavTenantPost {
        post_id: 0,
        title: "other-tenant".into(),
        blog_id,
        tenant_id: 2,
        comments: HasMany::new(),
    });
    ctx.save_changes().await.expect("insert posts");

    // Include loading should only return the tenant_id=1 post.
    let blogs = linq!(ctx.set::<NavTenantBlog>(); include b.posts)
        .to_list()
        .await
        .expect("include query");
    assert_eq!(blogs.len(), 1);
    let posts = blogs[0].posts.items();
    assert_eq!(
        posts.len(),
        1,
        "navigation should only contain same-tenant posts"
    );
    assert_eq!(posts[0].title, "own-tenant");
    assert_eq!(posts[0].tenant_id, 1);
}

#[tokio::test]
async fn hasmany_navigation_without_filter_returns_all() {
    // No query filter configured → navigation loads all children.
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.set::<NavTenantBlog>();
    ctx.set::<NavTenantPost>();
    ctx.set::<NavTenantComment>();
    ctx.ensure_created().await.expect("ensure_created");

    ctx.add::<NavTenantBlog>(NavTenantBlog {
        blog_id: 0,
        url: "https://nofilter.example".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await.expect("insert blog");
    let blog_id = ctx
        .set::<NavTenantBlog>()
        .query()
        .to_list()
        .await
        .expect("query blog")[0]
        .blog_id;

    ctx.add::<NavTenantPost>(NavTenantPost {
        post_id: 0,
        title: "t1".into(),
        blog_id,
        tenant_id: 1,
        comments: HasMany::new(),
    });
    ctx.add::<NavTenantPost>(NavTenantPost {
        post_id: 0,
        title: "t2".into(),
        blog_id,
        tenant_id: 2,
        comments: HasMany::new(),
    });
    ctx.save_changes().await.expect("insert posts");

    let blogs = linq!(ctx.set::<NavTenantBlog>(); include b.posts)
        .to_list()
        .await
        .expect("include query");
    assert_eq!(blogs.len(), 1);
    assert_eq!(
        blogs[0].posts.len(),
        2,
        "without filter, all posts should load"
    );
}

#[tokio::test]
async fn nested_include_respects_tenant_filter() {
    let mut ctx = make_ctx().await;

    ctx.add::<NavTenantBlog>(NavTenantBlog {
        blog_id: 0,
        url: "https://nested-tenant.example".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await.expect("insert blog");
    let blog_id = ctx
        .set::<NavTenantBlog>()
        .query()
        .to_list()
        .await
        .expect("query blog")[0]
        .blog_id;

    // Insert a same-tenant post.
    ctx.add::<NavTenantPost>(NavTenantPost {
        post_id: 0,
        title: "own-post".into(),
        blog_id,
        tenant_id: 1,
        comments: HasMany::new(),
    });
    ctx.save_changes().await.expect("insert post");
    let post_id = ctx
        .set::<NavTenantPost>()
        .query()
        .to_list()
        .await
        .expect("query post")[0]
        .post_id;

    // Insert comments across two tenants on the same post.
    ctx.add::<NavTenantComment>(NavTenantComment {
        comment_id: 0,
        text: "own-tenant-comment".into(),
        post_id,
        tenant_id: 1,
    });
    ctx.add::<NavTenantComment>(NavTenantComment {
        comment_id: 0,
        text: "other-tenant-comment".into(),
        post_id,
        tenant_id: 2,
    });
    ctx.save_changes().await.expect("insert comments");

    // Nested include: blog → posts → comments.
    // filter_map must be threaded through load_nested_includes so that
    // comments from other tenants are filtered out.
    let blogs = linq!(ctx.set::<NavTenantBlog>(); include b.posts then b.comments)
        .to_list()
        .await
        .expect("nested include query");

    assert_eq!(blogs.len(), 1);
    let posts = blogs[0].posts.items();
    assert_eq!(posts.len(), 1, "posts filtered to tenant_id=1");
    let comments = posts[0].comments.items();
    assert_eq!(
        comments.len(),
        1,
        "nested comments should be filtered to tenant_id=1"
    );
    assert_eq!(comments[0].text, "own-tenant-comment");
    assert_eq!(comments[0].tenant_id, 1);
}
