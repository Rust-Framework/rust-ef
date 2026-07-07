//! Tests for `linq!` Form B (multi-clause query) and Form C (value-producing).
//!
//! Covers clauses not already exercised by `linq_tests.rs` / `navigation_tests.rs` /
//! `sqlite_crud_tests.rs`: `order_by`, `group_by`, `select`, `having`, `min`/`max`
//! typed return (G1), `count`, `distinct`, `set`+`execute_update`, `take`/`skip`,
//! `inner_join`/`left_join`, and Form C `filter`/`index`/`key`.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("dsl_blogs")]
struct DslBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    title: String,
    rating: i32,
    views: i64,
    published: bool,
    category: String,
    #[navigation]
    posts: HasMany<DslPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("dsl_posts")]
struct DslPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(DslBlog)]
    blog_id: i32,
    #[navigation]
    blog: BelongsTo<DslBlog>,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Seeds three blogs across two categories with posts attached.
/// Returns the DbContext ready for querying.
async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<DslBlog>();
    ctx.set::<DslPost>();
    ctx.ensure_created().await.unwrap();

    ctx.set::<DslBlog>().add(DslBlog {
        blog_id: 0,
        title: "Rust".into(),
        rating: 9,
        views: 100,
        published: true,
        category: "tech".into(),
        posts: HasMany::new(),
    });
    ctx.set::<DslBlog>().add(DslBlog {
        blog_id: 0,
        title: "Async".into(),
        rating: 7,
        views: 50,
        published: true,
        category: "tech".into(),
        posts: HasMany::new(),
    });
    ctx.set::<DslBlog>().add(DslBlog {
        blog_id: 0,
        title: "Cooking".into(),
        rating: 3,
        views: 10,
        published: false,
        category: "food".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await.unwrap();

    let blogs = ctx.set::<DslBlog>().query().to_list().await.unwrap();
    let tech_id = blogs[0].blog_id;
    ctx.set::<DslPost>().add(DslPost {
        post_id: 0,
        title: "P1".into(),
        blog_id: tech_id,
        blog: BelongsTo::new(),
    });
    ctx.set::<DslPost>().add(DslPost {
        post_id: 0,
        title: "P2".into(),
        blog_id: tech_id,
        blog: BelongsTo::new(),
    });
    ctx.save_changes().await.unwrap();
    ctx
}

#[tokio::test]
async fn test_order_by_clause_desc() {
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<DslBlog>(); order_by b.rating desc)
        .to_list()
        .await
        .unwrap();
    assert_eq!(blogs.len(), 3);
    assert_eq!(blogs[0].rating, 9);
    assert_eq!(blogs[2].rating, 3);
}

#[tokio::test]
async fn test_order_by_clause_asc() {
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<DslBlog>(); order_by b.rating asc)
        .to_list()
        .await
        .unwrap();
    assert_eq!(blogs[0].rating, 3);
    assert_eq!(blogs[2].rating, 9);
}

#[tokio::test]
async fn test_take_skip_clauses() {
    let mut ctx = seed().await;
    let blogs = linq!(ctx.set::<DslBlog>(); order_by b.blog_id asc; skip 1; take 1)
        .to_list()
        .await
        .unwrap();
    assert_eq!(blogs.len(), 1);
    assert_eq!(blogs[0].title, "Async");
}

#[tokio::test]
async fn test_min_typed_return_i32() {
    // G1 verification: min returns typed Option<V>, not Option<String>.
    let mut ctx = seed().await;
    let min_rating: i32 = linq!(ctx.set::<DslBlog>(); min b.rating)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(min_rating, 3);
}

#[tokio::test]
async fn test_max_typed_return_i32() {
    // G1 verification: max returns typed Option<V>, not Option<String>.
    let mut ctx = seed().await;
    let max_rating: i32 = linq!(ctx.set::<DslBlog>(); max b.rating)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(max_rating, 9);
}

#[tokio::test]
async fn test_max_typed_return_i64() {
    // G1: cross-type inference — views column is i64.
    let mut ctx = seed().await;
    let max_views: i64 = linq!(ctx.set::<DslBlog>(); max b.views)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(max_views, 100);
}

#[tokio::test]
async fn test_min_empty_returns_none() {
    // G1: empty result set yields Ok(None), not an error.
    let mut ctx = make_ctx();
    ctx.set::<DslBlog>();
    ctx.ensure_created().await.unwrap();
    let min_rating: Option<i32> = linq!(ctx.set::<DslBlog>(); min b.rating).await.unwrap();
    assert!(min_rating.is_none());
}

#[tokio::test]
async fn test_count_clause() {
    let mut ctx = seed().await;
    let n: i64 = linq!(ctx.set::<DslBlog>(); count).await.unwrap();
    assert_eq!(n, 3);
}

#[tokio::test]
async fn test_distinct_clause() {
    let mut ctx = seed().await;
    // Distinct on the whole row; with a filter so the result is predictable.
    let rows = linq!(ctx.set::<DslBlog>(), |b: DslBlog| b.published; distinct)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
}

#[tokio::test]
async fn test_group_by_with_having() {
    let mut ctx = seed().await;
    // tech category has 2 blogs; food has 1. HAVING count > 1 keeps only tech.
    let blogs = linq!(ctx.set::<DslBlog>(); group_by b.category; having count(b.blog_id) > 1)
        .to_list()
        .await
        .unwrap();
    assert!(!blogs.is_empty());
}

#[tokio::test]
async fn test_having_with_and() {
    let mut ctx = seed().await;
    // tech: count=2 > 1 AND sum(views)=150 > 100 → true
    // food: count=1 > 1 → false (short-circuit) → excluded
    let blogs = linq!(
        ctx.set::<DslBlog>();
        group_by b.category;
        having count(b.blog_id) > 1 && sum(b.views) > 100
    )
    .to_list()
    .await
    .unwrap();
    assert_eq!(blogs.len(), 1, "only tech group should pass AND condition");
    assert_eq!(blogs[0].category, "tech");
}

#[tokio::test]
async fn test_having_with_or() {
    let mut ctx = seed().await;
    // tech: count=2 > 5 → false; sum(views)=150 > 100 → true → true
    // food: count=1 > 5 → false; sum(views)=10 > 100 → false → false
    let blogs = linq!(
        ctx.set::<DslBlog>();
        group_by b.category;
        having count(b.blog_id) > 5 || sum(b.views) > 100
    )
    .to_list()
    .await
    .unwrap();
    assert_eq!(blogs.len(), 1, "only tech group should pass OR condition");
    assert_eq!(blogs[0].category, "tech");
}

#[tokio::test]
async fn test_having_with_not() {
    let mut ctx = seed().await;
    // NOT(count > 1): tech (NOT true = false), food (NOT false = true)
    let blogs = linq!(
        ctx.set::<DslBlog>();
        group_by b.category;
        having !(count(b.blog_id) > 1)
    )
    .to_list()
    .await
    .unwrap();
    assert_eq!(blogs.len(), 1, "only food group should pass NOT condition");
    assert_eq!(blogs[0].category, "food");
}

#[tokio::test]
async fn test_having_compare_agg() {
    let mut ctx = seed().await;
    // sum(views) > count(blog_id):
    //   tech: 150 > 2 → true
    //   food: 10 > 1 → true
    let blogs = linq!(
        ctx.set::<DslBlog>();
        group_by b.category;
        having sum(b.views) > count(b.blog_id)
    )
    .to_list()
    .await
    .unwrap();
    assert_eq!(
        blogs.len(),
        2,
        "both groups should pass agg-vs-agg comparison"
    );
}

#[tokio::test]
async fn test_having_nested_and_or() {
    let mut ctx = seed().await;
    // count > 1 && (sum(views) > 100 || count(blog_id) > 0)
    //   tech: true && (true || true) → true
    //   food: false && ... → false
    let blogs = linq!(
        ctx.set::<DslBlog>();
        group_by b.category;
        having count(b.blog_id) > 1 && (sum(b.views) > 100 || count(b.blog_id) > 0)
    )
    .to_list()
    .await
    .unwrap();
    assert_eq!(blogs.len(), 1, "only tech should pass nested AND/OR");
    assert_eq!(blogs[0].category, "tech");
}

#[tokio::test]
async fn test_select_clause_returns_raw_rows() {
    let mut ctx = seed().await;
    let rows: Vec<Vec<String>> =
        linq!(ctx.set::<DslBlog>(), |b: DslBlog| b.published; select (b.blog_id, b.title))
            .to_list()
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].len(), 2);
}

#[tokio::test]
async fn test_set_execute_update() {
    let mut ctx = seed().await;
    let affected = linq!(ctx.set::<DslBlog>(), |b: DslBlog| b.rating < 5; set b.published, false; execute_update)
        .await
        .unwrap();
    assert_eq!(affected, 1);

    // Verify the update took effect.
    let cooking = linq!(ctx.set::<DslBlog>(), |b: DslBlog| b.title == "Cooking")
        .to_list()
        .await
        .unwrap();
    assert_eq!(cooking.len(), 1);
    assert!(!cooking[0].published);
}

#[tokio::test]
async fn test_inner_join_clause() {
    let mut ctx = seed().await;
    let blogs =
        linq!(ctx.set::<DslBlog>(); inner_join |a: DslBlog, b: DslPost| a.blog_id == b.blog_id)
            .to_list()
            .await
            .unwrap();
    // Two posts belong to the first blog; inner join yields rows for those.
    assert!(!blogs.is_empty());
}

#[tokio::test]
async fn test_left_join_clause() {
    let mut ctx = seed().await;
    let blogs =
        linq!(ctx.set::<DslBlog>(); left_join |a: DslBlog, b: DslPost| a.blog_id == b.blog_id)
            .to_list()
            .await
            .unwrap();
    // Left join keeps all blogs; blog 1 appears twice (one row per matching post),
    // blogs 2 and 3 appear once each with NULL post side → 4 rows total.
    assert!(blogs.len() >= 3);
}

#[tokio::test]
async fn test_right_join_clause() {
    let mut ctx = seed().await;
    // RIGHT JOIN keeps all posts (right side), even if blog missing.
    // Both posts belong to blog 1, so result has 2 rows.
    let rows =
        linq!(ctx.set::<DslBlog>(); right_join |a: DslBlog, b: DslPost| a.blog_id == b.blog_id)
            .to_list()
            .await
            .unwrap();
    assert_eq!(rows.len(), 2, "right join should yield one row per post");
}

#[tokio::test]
async fn test_full_join_clause() {
    let mut ctx = seed().await;
    // FULL JOIN keeps all blogs and all posts. Blog 1 matches 2 posts,
    // blogs 2 and 3 have no matching posts → 4 rows total.
    let rows =
        linq!(ctx.set::<DslBlog>(); full_join |a: DslBlog, b: DslPost| a.blog_id == b.blog_id)
            .to_list()
            .await
            .unwrap();
    assert_eq!(rows.len(), 4, "full join should yield blogs + posts");
}

#[tokio::test]
async fn test_cross_join_clause() {
    let mut ctx = seed().await;
    // CROSS JOIN is the cartesian product: 3 blogs × 2 posts = 6 rows.
    let rows = linq!(ctx.set::<DslBlog>(); cross_join b: DslPost)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 6, "cross join should yield cartesian product");
}

#[tokio::test]
async fn test_union_clause() {
    let mut ctx = seed().await;
    // Main: all blogs (3 rows). Operand: all blogs (3 rows, identical).
    // UNION dedupes identical rows → 3 rows.
    let op = ctx.set::<DslBlog>().query().compile_sql();
    let rows = linq!(ctx.set::<DslBlog>(); union op)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3, "UNION should dedupe identical rows");
}

#[tokio::test]
async fn test_union_all_clause() {
    let mut ctx = seed().await;
    // Main: all blogs (3 rows). Operand: all blogs (3 rows, identical).
    // UNION ALL does not dedupe → 6 rows.
    let op = ctx.set::<DslBlog>().query().compile_sql();
    let rows = linq!(ctx.set::<DslBlog>(); union_all op)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 6, "UNION ALL should not dedupe");
}

#[tokio::test]
async fn test_except_clause() {
    let mut ctx = seed().await;
    // Main: all blogs (3 rows). EXCEPT operand: tech blogs (2 rows).
    // Result: 1 row (Cooking, the food blog).
    let op = linq!(ctx.set::<DslBlog>(), |b: DslBlog| b.category == "tech")
        .compile_sql();
    let rows = linq!(ctx.set::<DslBlog>(); except op)
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "EXCEPT should yield the difference");
    assert_eq!(rows[0].category, "food");
}

#[tokio::test]
async fn test_form_c_filter_produces_bool_expr() {
    use rust_ef::query::BoolExpr;
    let expr: BoolExpr = linq!(filter |b: DslBlog| b.published);
    // The expression is a value, not a method chain; just verify it's not Filter-free.
    assert!(matches!(expr, BoolExpr::Filter(_)));
}

#[tokio::test]
async fn test_form_c_index_produces_str_slice() {
    let cols: &'static [&'static str] = linq!(index |b: DslBlog| (b.category, b.rating));
    assert_eq!(cols.len(), 2);
    assert!(cols.contains(&"category"));
    assert!(cols.contains(&"rating"));
}

#[tokio::test]
async fn test_form_c_key_produces_str_slice() {
    let cols: &'static [&'static str] = linq!(key |b: DslBlog| b.blog_id);
    assert_eq!(cols.len(), 1);
    assert_eq!(cols[0], "blog_id");
}
