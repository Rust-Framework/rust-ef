//! Composable predicates: `Predicate<T>` + `QueryBuilder::where_`.
//!
//! Covers the EF Core `PredicateBuilder`-style workflow — seed an `AND`/`OR`
//! chain, combine `linq!(filter ...)` fragments with `and` / `or`, then apply
//! the result to a query — at the SQL level (grouping and parameter order) and
//! against a real SQLite database.

use std::collections::HashSet;

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::{Predicate, QueryBuilder, QueryState};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("predicate_products")]
struct Product {
    #[primary_key]
    id: i32,
    name: String,
    description: String,
    price: f64,
    is_deleted: bool,
}

// ---------------------------------------------------------------------------
// SQL shape (no database)
// ---------------------------------------------------------------------------

#[test]
fn always_seed_adds_no_sql() {
    let query =
        QueryBuilder::<Product>::new("predicate_products").where_(Predicate::<Product>::always());

    assert_eq!(query.to_sql(), "SELECT * FROM predicate_products");
    assert!(query.compile_sql().1.is_empty());
}

#[test]
fn or_group_keeps_its_parentheses_and_value_order() {
    let keyword = "rust";
    let predicate = Predicate::<Product>::always()
        .and(linq!(filter |p: Product| !p.is_deleted))
        .and(
            linq!(filter |p: Product| p.name.contains(keyword))
                .or(linq!(filter |p: Product| p.description.contains(keyword))),
        )
        .and(linq!(filter |p: Product| p.price >= 9.9));

    let (sql, params) = QueryBuilder::<Product>::new("predicate_products")
        .where_(predicate)
        .compile_sql();

    assert!(
        sql.contains("(name LIKE ?) OR (description LIKE ?)"),
        "OR group must stay grouped: {sql}"
    );
    assert_eq!(
        params,
        vec![
            DbValue::Bool(true),
            DbValue::String("%rust%".into()),
            DbValue::String("%rust%".into()),
            DbValue::F64(9.9),
        ],
        "values must follow tree order"
    );
}

#[test]
fn never_seed_disappears_from_an_or_chain() {
    let predicate = Predicate::<Product>::never()
        .or(linq!(filter |p: Product| p.name.contains("a")))
        .or(linq!(filter |p: Product| p.description.contains("b")));

    let (sql, params) = QueryBuilder::<Product>::new("predicate_products")
        .where_(predicate)
        .compile_sql();

    assert!(!sql.contains("1=0"), "seed must not leak into SQL: {sql}");
    assert!(
        sql.contains("(name LIKE ?) OR (description LIKE ?)"),
        "{sql}"
    );
    assert_eq!(params.len(), 2);
}

#[test]
fn never_applied_on_its_own_matches_nothing() {
    let (sql, params) = QueryBuilder::<Product>::new("predicate_products")
        .where_(Predicate::<Product>::never())
        .compile_sql();

    assert!(sql.contains("1=0"), "{sql}");
    assert!(params.is_empty());
}

#[test]
fn where_appends_to_conditions_added_earlier() {
    let query = QueryBuilder::<Product>::new("predicate_products")
        .filter(linq!(|p: Product| p.price > 1.0))
        .where_(linq!(filter |p: Product| p.name.contains("x")));

    let (sql, params) = query.compile_sql();

    assert!(sql.contains("(price > ?) AND (name LIKE ?)"), "{sql}");
    assert_eq!(
        params,
        vec![DbValue::F64(1.0), DbValue::String("%x%".into())]
    );
}

#[test]
fn predicate_round_trips_to_bool_expr() {
    let predicate: Predicate<Product> = linq!(filter |p: Product| p.price > 0.0).into();

    let mut state = QueryState::new("predicate_products");
    state.where_expr = Some(predicate.into_expr());

    assert!(state.to_sql().contains("price > ?"));
}

// ---------------------------------------------------------------------------
// Execution against SQLite
// ---------------------------------------------------------------------------

async fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.set::<Product>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

async fn seed(ctx: &mut DbContext) {
    let products = [
        (1, "rust book", "systems programming", 30.0, false),
        (2, "go book", "rust in action", 20.0, false),
        (3, "rust cheap", "nothing", 5.0, false),
        (4, "rust deleted", "rust everywhere", 40.0, true),
    ];
    for (id, name, description, price, is_deleted) in products {
        ctx.add::<Product>(Product {
            id,
            name: name.into(),
            description: description.into(),
            price,
            is_deleted,
        });
    }
    ctx.save_changes().await.expect("seed");
}

/// Mirrors the EF Core `PredicateBuilder` example: a soft-delete base filter,
/// an `OR` group over two columns, and an optional range condition.
fn product_filter(keyword: &str, min_price: Option<f64>) -> Predicate<Product> {
    let mut predicate = Predicate::<Product>::always()
        .and(linq!(filter |p: Product| !p.is_deleted))
        .and(
            linq!(filter |p: Product| p.name.contains(keyword))
                .or(linq!(filter |p: Product| p.description.contains(keyword))),
        );

    if let Some(min_price) = min_price {
        predicate = predicate.and(linq!(filter |p: Product| p.price >= min_price));
    }
    predicate
}

async fn matching_ids(predicate: Predicate<Product>) -> Vec<i32> {
    let mut ctx = make_ctx().await;
    seed(&mut ctx).await;
    let rows = ctx
        .set::<Product>()
        .query()
        .where_(predicate)
        .to_list()
        .await
        .expect("query");
    rows.into_iter().map(|p| p.id).collect()
}

#[tokio::test]
async fn keyword_or_group_matches_either_column() {
    let ids = matching_ids(product_filter("rust", None)).await;
    assert_eq!(ids, vec![1, 2, 3], "soft-deleted row 4 must stay filtered");
}

#[tokio::test]
async fn optional_range_condition_narrows_the_same_predicate() {
    let ids = matching_ids(product_filter("rust", Some(10.0))).await;
    assert_eq!(ids, vec![1, 2]);
}

#[tokio::test]
async fn count_and_page_share_one_predicate() {
    let mut ctx = make_ctx().await;
    seed(&mut ctx).await;

    let base = ctx
        .set::<Product>()
        .query()
        .where_(product_filter("rust", None));

    let total = base.clone().count().await.expect("count");
    let rows = base.to_list().await.expect("list");

    assert_eq!(total, 3);
    assert_eq!(rows.len() as i64, total);
    let ids: HashSet<i32> = rows.into_iter().map(|p| p.id).collect();
    assert_eq!(ids, HashSet::from([1, 2, 3]));
}

#[tokio::test]
async fn never_predicate_returns_no_rows() {
    let ids = matching_ids(Predicate::<Product>::never()).await;
    assert!(ids.is_empty());
}
