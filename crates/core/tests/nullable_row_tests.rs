//! Nullable projection cells: a `#[derive(FromRow)]` struct / tuple can carry
//! `Option<T>` for SQL `NULL` (LEFT JOIN misses, nullable report columns).
//!
//! `DbValue::Null` decodes to `None`; any other cell decodes through the inner
//! type's `TryFrom<DbValue>`. This is the raw-SQL + `sql_query_rows` path that
//! report/LEFT JOIN projections use.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef::FromRow;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("nr_authors")]
struct NrAuthor {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
    #[navigation]
    posts: HasMany<NrPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("nr_posts")]
struct NrPost {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    title: String,
    #[foreign_key(NrAuthor)]
    author_id: i32,
    #[required]
    views: i64,
    #[navigation]
    author: BelongsTo<NrAuthor>,
}

#[derive(Debug, FromRow)]
struct AuthorPostRow {
    author_name: String,
    post_title: Option<String>,
    post_views: Option<i64>,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<NrAuthor>();
    ctx.set::<NrPost>();
    ctx.ensure_created().await.unwrap();

    // Two authors: the first has one post, the second has none.
    for name in ["alice", "bob"] {
        ctx.add::<NrAuthor>(NrAuthor {
            id: 0,
            name: name.to_string(),
            posts: HasMany::new(),
        });
        ctx.save_changes().await.unwrap();
    }
    let authors = ctx.set::<NrAuthor>().query().to_list().await.unwrap();
    let alice_id = authors[0].id;
    ctx.add::<NrPost>(NrPost {
        id: 0,
        title: "hello".to_string(),
        author_id: alice_id,
        views: 42,
        author: BelongsTo::new(),
    });
    ctx.save_changes().await.unwrap();
    ctx
}

#[tokio::test]
async fn left_join_misses_decode_into_none() {
    let ctx = seed().await;

    let sql = "SELECT a.name AS author_name, p.title AS post_title, p.views AS post_views \
               FROM nr_authors a LEFT JOIN nr_posts p ON p.author_id = a.id ORDER BY a.id";
    let rows: Vec<AuthorPostRow> = ctx.sql_query_rows(sql, &[]).await.unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].author_name, "alice");
    assert_eq!(rows[0].post_title.as_deref(), Some("hello"));
    assert_eq!(rows[0].post_views, Some(42));
    assert_eq!(rows[1].author_name, "bob");
    assert_eq!(rows[1].post_title, None);
    assert_eq!(rows[1].post_views, None);
}

#[tokio::test]
async fn a_tuple_row_also_accepts_optional_cells() {
    let ctx = seed().await;

    let sql = "SELECT p.title, p.views FROM nr_authors a \
               LEFT JOIN nr_posts p ON p.author_id = a.id ORDER BY a.id";
    let rows: Vec<(Option<String>, Option<i64>)> = ctx.sql_query_rows(sql, &[]).await.unwrap();

    assert_eq!(
        rows,
        vec![(Some("hello".to_string()), Some(42)), (None, None),]
    );
}

#[tokio::test]
async fn a_fractional_numeric_is_not_silently_truncated_into_an_integer() {
    // `SUM` over an empty set is SQL NULL (decoded as `None` here); a
    // fractional value decoded into `i64` is a typed conversion error rather
    // than a silent truncation.
    let ctx = seed().await;

    let rows: Vec<(Option<i64>,)> = ctx
        .sql_query_rows("SELECT SUM(views) FROM nr_posts WHERE views < 0", &[])
        .await
        .unwrap();
    assert_eq!(rows, vec![(None,)]);

    let result: rust_ef::error::EFResult<Vec<(i64,)>> = ctx
        .sql_query_rows("SELECT AVG(views + 0.5) FROM nr_posts", &[])
        .await;
    assert!(result.is_err(), "fractional AVG must not decode into i64");
}
