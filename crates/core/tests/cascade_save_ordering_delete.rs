//! Cascade save tests: update ordering and cascade delete variants.

mod common;

use common::*;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::test]
async fn cascade_update_ordering() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeBlog>();
    ctx.set::<CascadePost>();
    ctx.ensure_created().await.unwrap();

    // Insert blog with one post
    let blog = CascadeBlog {
        blog_id: 0,
        url: "https://original.example".into(),
        posts: HasMany::with(vec![CascadePost {
            post_id: 0,
            title: "Original Title".into(),
            blog_id: 0,
            blog: BelongsTo::new(),
        }]),
    };
    ctx.set::<CascadeBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    // Query back and modify
    let mut blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
    let blog_id = blogs[0].blog_id;
    blogs[0].url = "https://updated.example".into();
    ctx.set::<CascadeBlog>()
        .update(blogs.into_iter().next().unwrap());

    let mut posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
    posts[0].title = "Updated Title".into();
    ctx.set::<CascadePost>()
        .update(posts.into_iter().next().unwrap());

    ctx.save_changes().await.unwrap();

    // Verify updates
    let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
    assert_eq!(blogs[0].url, "https://updated.example");

    let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
    assert_eq!(posts[0].title, "Updated Title");
    assert_eq!(posts[0].blog_id, blog_id);
}

#[tokio::test]
async fn cascade_delete_reverse_order() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeBlog>();
    ctx.set::<CascadePost>();
    ctx.ensure_created().await.unwrap();

    // Insert blog with one post
    let blog = CascadeBlog {
        blog_id: 0,
        url: "https://delete.example".into(),
        posts: HasMany::with(vec![CascadePost {
            post_id: 0,
            title: "To Delete".into(),
            blog_id: 0,
            blog: BelongsTo::new(),
        }]),
    };
    ctx.set::<CascadeBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    // Mark entries for deletion (Post first, then Blog — reverse topo order
    // is handled by the save pipeline)
    ctx.set::<CascadePost>().remove_at(0).unwrap();
    ctx.set::<CascadeBlog>().remove_at(0).unwrap();
    ctx.save_changes().await.unwrap();

    // Verify tables are empty
    let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
    assert!(blogs.is_empty(), "Blog table should be empty after delete");
    let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
    assert!(posts.is_empty(), "Post table should be empty after delete");
}

#[tokio::test]
async fn cascade_delete_loaded_children() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeBlog>();
    ctx.set::<CascadePost>();
    ctx.ensure_created().await.unwrap();

    let blog = CascadeBlog {
        blog_id: 0,
        url: "https://loaded-delete.example".into(),
        posts: HasMany::with(vec![
            CascadePost {
                post_id: 0,
                title: "Post A".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            },
            CascadePost {
                post_id: 0,
                title: "Post B".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            },
        ]),
    };
    ctx.set::<CascadeBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    // Re-query with include to populate HasMany, then mark Deleted
    ctx.set::<CascadeBlog>().clear_entries();
    let loaded = ctx
        .set::<CascadeBlog>()
        .query()
        .include_internal("posts")
        .to_list()
        .await
        .unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(
        loaded[0].posts.len(),
        2,
        "Posts should be loaded via include"
    );

    ctx.set::<CascadeBlog>()
        .attach(loaded.into_iter().next().unwrap());
    ctx.set::<CascadeBlog>().remove_at(0).unwrap();
    ctx.save_changes().await.unwrap();

    let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
    assert!(blogs.is_empty(), "Blog table should be empty");
    let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
    assert!(
        posts.is_empty(),
        "Post table should be empty (cascade delete)"
    );
}

#[tokio::test]
async fn cascade_delete_untracked_children() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeBlog>();
    ctx.set::<CascadePost>();
    ctx.ensure_created().await.unwrap();

    let blog = CascadeBlog {
        blog_id: 0,
        url: "https://untracked-delete.example".into(),
        posts: HasMany::with(vec![
            CascadePost {
                post_id: 0,
                title: "Untracked A".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            },
            CascadePost {
                post_id: 0,
                title: "Untracked B".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            },
        ]),
    };
    ctx.set::<CascadeBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    // Mark blog Deleted without loading posts — direct DELETE SQL handles them
    ctx.set::<CascadeBlog>().remove_at(0).unwrap();
    ctx.save_changes().await.unwrap();

    let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
    assert!(blogs.is_empty(), "Blog table should be empty");
    let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
    assert!(
        posts.is_empty(),
        "Post table should be empty (direct DELETE SQL)"
    );
}

#[tokio::test]
async fn cascade_delete_nested() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeNestBlog>();
    ctx.set::<CascadeNestPost>();
    ctx.set::<CascadeNestComment>();
    ctx.ensure_created().await.unwrap();

    let blog = CascadeNestBlog {
        blog_id: 0,
        url: "https://nested.example".into(),
        posts: HasMany::with(vec![CascadeNestPost {
            post_id: 0,
            title: "Nested Post".into(),
            blog_id: 0,
            blog: BelongsTo::new(),
            comments: HasMany::with(vec![CascadeNestComment {
                comment_id: 0,
                text: "Nested Comment".into(),
                post_id: 0,
            }]),
        }]),
    };
    ctx.set::<CascadeNestBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    // Re-query with nested include, then mark Deleted
    ctx.set::<CascadeNestBlog>().clear_entries();
    let loaded = ctx
        .set::<CascadeNestBlog>()
        .query()
        .include_internal("posts")
        .then_include_internal("comments")
        .to_list()
        .await
        .unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].posts.len(), 1);
    assert_eq!(loaded[0].posts.items()[0].comments.len(), 1);

    ctx.set::<CascadeNestBlog>()
        .attach(loaded.into_iter().next().unwrap());
    ctx.set::<CascadeNestBlog>().remove_at(0).unwrap();
    ctx.save_changes().await.unwrap();

    let blogs = ctx
        .set::<CascadeNestBlog>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert!(blogs.is_empty(), "Blog table should be empty");
    let posts = ctx
        .set::<CascadeNestPost>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert!(posts.is_empty(), "Post table should be empty");
    let comments = ctx
        .set::<CascadeNestComment>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert!(comments.is_empty(), "Comment table should be empty");
}

#[tokio::test]
async fn cascade_delete_set_null() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeOptionalBlog>();
    ctx.set::<CascadeOptionalPost>();
    ctx.ensure_created().await.unwrap();

    let blog = CascadeOptionalBlog {
        blog_id: 0,
        url: "https://setnull.example".into(),
        posts: HasMany::with(vec![CascadeOptionalPost {
            post_id: 0,
            title: "SetNull Post".into(),
            blog_id: None,
            blog: BelongsTo::new(),
        }]),
    };
    ctx.set::<CascadeOptionalBlog>().add(blog);
    ctx.save_changes().await.unwrap();

    // Mark blog Deleted — SetNull should nullify FK, post should survive
    ctx.set::<CascadeOptionalBlog>().remove_at(0).unwrap();
    ctx.save_changes().await.unwrap();

    let blogs = ctx
        .set::<CascadeOptionalBlog>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert!(blogs.is_empty(), "Blog table should be empty");

    let posts = ctx
        .set::<CascadeOptionalPost>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert_eq!(posts.len(), 1, "Post should be preserved (SetNull)");
    assert!(
        posts[0].blog_id.is_none(),
        "Post blog_id should be NULL after SetNull"
    );
}
