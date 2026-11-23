//! A projected query must still resolve correlated subqueries.
//!
//! `SelectQueryBuilder` compiles its own SQL (it does not go through
//! `QueryBuilder::to_list`), so the `EXISTS` / `IN (SELECT ...)` predicate
//! nodes have to be resolved from the root entity's navigation metadata before
//! the SELECT is emitted. Regression test for a 2.6-era gap that the typed
//! report path made easy to hit.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("proj_blogs")]
struct ProjBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    url: String,
    #[navigation]
    posts: HasMany<ProjPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("proj_posts")]
struct ProjPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(ProjBlog)]
    blog_id: i32,
    published: bool,
    #[navigation]
    blog: BelongsTo<ProjBlog>,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<ProjBlog>();
    ctx.set::<ProjPost>();
    ctx.ensure_created().await.unwrap();

    for (url, post) in [
        ("https://p1.example", Some(("published post", true))),
        ("https://p2.example", Some(("draft post", false))),
        ("https://p3.example", None),
    ] {
        ctx.add::<ProjBlog>(ProjBlog {
            blog_id: 0,
            url: url.to_string(),
            posts: HasMany::new(),
        });
        ctx.save_changes().await.unwrap();
        let blogs = ctx.set::<ProjBlog>().query().to_list().await.unwrap();
        let blog_id = blogs.last().unwrap().blog_id;

        if let Some((title, published)) = post {
            ctx.add::<ProjPost>(ProjPost {
                post_id: 0,
                title: title.to_string(),
                blog_id,
                published,
                blog: BelongsTo::new(),
            });
            ctx.save_changes().await.unwrap();
        }
    }
    ctx
}

#[tokio::test]
async fn a_projection_resolves_the_correlated_subquery_before_compiling() {
    let mut ctx = seed().await;

    let query = linq!(
        ctx.set::<ProjBlog>(),
        |b: ProjBlog| b.posts.any(|p: ProjPost| p.published);
        select (b.blog_id, b.url)
    );
    let sql = query.to_sql();
    assert!(
        sql.contains("EXISTS (SELECT 1 FROM \"proj_posts\" WHERE \"proj_posts\".\"blog_id\" = \"proj_blogs\".\"blog_id\""),
        "the subquery tables must be resolved, got: {sql}"
    );

    let rows: Vec<(i32, String)> = query.to_list_typed_2().await.unwrap();
    assert_eq!(rows.len(), 1, "only the blog with a published post");
    assert_eq!(rows[0].1, "https://p1.example");
}

#[tokio::test]
async fn a_grouped_projection_resolves_the_subquery_too() {
    let mut ctx = seed().await;

    let rows: Vec<(String, i64)> = linq!(
        ctx.set::<ProjBlog>(),
        |b: ProjBlog| b.posts.any(|p: ProjPost| p.published);
        group_by b.url;
        select (b.url, count())
    )
    .to_list_typed_2()
    .await
    .unwrap();

    assert_eq!(rows, vec![("https://p1.example".to_string(), 1)]);
}
