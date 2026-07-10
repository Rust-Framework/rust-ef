//! Cascade save tests: one-to-many PK backfill and empty HasMany noop.

mod common;

use common::*;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::test]
async fn cascade_insert_blog_with_posts() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeBlog>();
    ctx.set::<CascadePost>();
    ctx.ensure_created().await.unwrap();

    let blog = CascadeBlog {
        blog_id: 0,
        url: "https://cascade.example".into(),
        posts: HasMany::with(vec![
            CascadePost {
                post_id: 0,
                title: "First Post".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            },
            CascadePost {
                post_id: 0,
                title: "Second Post".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            },
        ]),
    };
    ctx.set::<CascadeBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
    assert_eq!(blogs.len(), 1);
    let blog_id = blogs[0].blog_id;
    assert!(blog_id > 0, "Blog PK should be backfilled");

    let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
    assert_eq!(posts.len(), 2, "Two posts should be cascade-inserted");
    for post in &posts {
        assert_eq!(
            post.blog_id, blog_id,
            "Post blog_id should be fixed up to parent PK"
        );
    }
}

#[tokio::test]
async fn cascade_empty_has_many_noop() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeBlog>();
    ctx.set::<CascadePost>();
    ctx.ensure_created().await.unwrap();

    let blog = CascadeBlog {
        blog_id: 0,
        url: "https://empty.example".into(),
        posts: HasMany::new(),
    };
    ctx.set::<CascadeBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
    assert_eq!(blogs.len(), 1);
    assert!(blogs[0].blog_id > 0, "Blog PK should be backfilled");

    let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
    assert!(posts.is_empty(), "No posts should exist");
}
