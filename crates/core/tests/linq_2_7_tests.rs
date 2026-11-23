//! `linq!` macro coverage for rust-ef 2.7 additions:
//!
//! - composite / chained JOIN clauses (EF-04),
//! - `set` clauses whose RHS is a column arithmetic / another column (EF-03),
//! - the integer-exact `sum_i64` terminal (EF-02),
//! - `select` with aggregate items combined with `group_by` / `having` /
//!   `order_by` / `take` (EF-01).
//!
//! Every case is executed against an in-memory SQLite database so the macro
//! expansion and the generated SQL are validated end to end.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, EntityType)]
#[table("lq_items")]
struct LqItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    region_code: String,
    category: String,
    amount: i64,
    views: i64,
    stock: i64,
    reserved: i64,
    name: String,
}

/// Price list keyed by the composite key (`region_code`, `category`).
#[derive(Debug, Clone, EntityType)]
#[table("lq_prices")]
struct LqPrice {
    #[primary_key]
    region_code: String,
    #[primary_key]
    category: String,
    price: i64,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<LqItem>();
    ctx.set::<LqPrice>();
    ctx.ensure_created().await.unwrap();

    for (region_code, category, amount, views, stock, reserved, name) in [
        ("EU", "hardware", 10_i64, 100_i64, 10_i64, 2_i64, "A"),
        ("EU", "software", 20, 200, 20, 3, "B"),
        ("US", "hardware", 5, 50, 30, 4, "C"),
    ] {
        ctx.add::<LqItem>(LqItem {
            id: 0,
            region_code: region_code.into(),
            category: category.into(),
            amount,
            views,
            stock,
            reserved,
            name: name.into(),
        });
    }
    for (region_code, category, price) in [
        ("EU", "hardware", 100_i64),
        ("EU", "software", 200),
        ("US", "hardware", 300),
        ("US", "software", 400),
    ] {
        ctx.add::<LqPrice>(LqPrice {
            region_code: region_code.into(),
            category: category.into(),
            price,
        });
    }
    ctx.save_changes().await.unwrap();
    ctx
}

fn as_i64(value: &DbValue) -> i64 {
    match value {
        DbValue::I64(n) => *n,
        DbValue::I32(n) => i64::from(*n),
        other => panic!("expected an integer cell, got {other:?}"),
    }
}

fn as_string(value: &DbValue) -> String {
    match value {
        DbValue::String(s) => s.clone(),
        other => panic!("expected a string cell, got {other:?}"),
    }
}

async fn read_item(ctx: &mut DbContext, id: i32) -> LqItem {
    ctx.set::<LqItem>()
        .query()
        .filter(linq!(|i: LqItem| i.id == id))
        .first_or_default()
        .await
        .unwrap()
        .expect("item exists")
}

// ---------------------------------------------------------------------------
// Composite JOIN via `linq!`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn composite_join_clause_runs() {
    let mut ctx = seed().await;

    let items = linq!(
        ctx.set::<LqItem>();
        inner_join |i: LqItem, p: LqPrice| i.region_code == p.region_code
            && i.category == p.category
    )
    .to_list()
    .await
    .unwrap();
    assert_eq!(items.len(), 3, "every item has a matching composite price");

    // JOIN + filter_joined: keep items whose price is above 150.
    let expensive = linq!(
        ctx.set::<LqItem>();
        inner_join |i: LqItem, p: LqPrice| i.region_code == p.region_code
            && i.category == p.category;
    )
    .filter_joined(linq!(|p: LqPrice| p.price > 150))
    .to_list()
    .await
    .unwrap();
    let mut names: Vec<String> = expensive.iter().map(|i| i.name.clone()).collect();
    names.sort();
    assert_eq!(names, vec!["B".to_string(), "C".to_string()]);
}

// ---------------------------------------------------------------------------
// `set` expressions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn set_increment_expression_runs() {
    let mut ctx = seed().await;

    let affected = linq!(
        ctx.set::<LqItem>(), |i: LqItem| i.id == 1;
        set i.views, i.views + 10;
        execute_update
    )
    .await
    .unwrap();
    assert_eq!(affected, 1);

    let item = read_item(&mut ctx, 1).await;
    assert_eq!(item.views, 110, "views = views + 10");
    assert_eq!(item.amount, 10, "other columns unchanged");
}

#[tokio::test]
async fn set_decrement_expression_runs() {
    let mut ctx = seed().await;

    let affected = linq!(
        ctx.set::<LqItem>(), |i: LqItem| i.id == 1;
        set i.stock, i.stock - 3;
        execute_update
    )
    .await
    .unwrap();
    assert_eq!(affected, 1);

    let item = read_item(&mut ctx, 1).await;
    assert_eq!(item.stock, 7, "stock = stock - 3");
}

#[tokio::test]
async fn set_column_to_column_arithmetic_runs() {
    let mut ctx = seed().await;

    // stock = stock + reserved
    let affected = linq!(
        ctx.set::<LqItem>(), |i: LqItem| i.id == 2;
        set i.stock, i.stock + i.reserved;
        execute_update
    )
    .await
    .unwrap();
    assert_eq!(affected, 1);

    let item = read_item(&mut ctx, 2).await;
    assert_eq!(item.stock, 23, "20 + 3");

    // stock = reserved (copy another column)
    let affected = linq!(
        ctx.set::<LqItem>(), |i: LqItem| i.id == 3;
        set i.stock, i.reserved;
        execute_update
    )
    .await
    .unwrap();
    assert_eq!(affected, 1);

    let item = read_item(&mut ctx, 3).await;
    assert_eq!(item.stock, 4, "stock copied from reserved");
}

#[tokio::test]
async fn set_plain_value_still_runs() {
    let mut ctx = seed().await;

    let affected = linq!(
        ctx.set::<LqItem>(), |i: LqItem| i.id == 1;
        set i.name, "renamed";
        set i.views, 7;
        execute_update
    )
    .await
    .unwrap();
    assert_eq!(affected, 1);

    let item = read_item(&mut ctx, 1).await;
    assert_eq!(item.name, "renamed");
    assert_eq!(item.views, 7);
}

// ---------------------------------------------------------------------------
// `sum_i64` terminal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sum_i64_terminal_returns_exact_integer() {
    let mut ctx = seed().await;

    let total: i64 = linq!(ctx.set::<LqItem>(); sum_i64 i.amount).await.unwrap();
    assert_eq!(total, 35, "10 + 20 + 5");

    // Empty set → neutral value 0, not an error.
    let empty: i64 = linq!(ctx.set::<LqItem>(), |i: LqItem| i.id == 999; sum_i64 i.amount)
        .await
        .unwrap();
    assert_eq!(empty, 0);
}

// ---------------------------------------------------------------------------
// `select` aggregate items + group_by / having / order_by / take
// ---------------------------------------------------------------------------

#[tokio::test]
async fn select_sum_and_count_with_group_by() {
    let mut ctx = seed().await;

    let rows: Vec<Vec<DbValue>> = linq!(
        ctx.set::<LqItem>();
        group_by i.category;
        select (i.category, sum(i.amount), count())
    )
    .to_list()
    .await
    .unwrap();

    let mut by_category: std::collections::HashMap<String, (i64, i64)> = rows
        .iter()
        .map(|row| (as_string(&row[0]), (as_i64(&row[1]), as_i64(&row[2]))))
        .collect();
    assert_eq!(by_category.len(), 2);
    assert_eq!(by_category.remove("hardware"), Some((15, 2)));
    assert_eq!(by_category.remove("software"), Some((20, 1)));
}

#[tokio::test]
async fn select_before_group_by_is_supported() {
    let mut ctx = seed().await;

    // `select` before `group_by` must compile (SelectQueryBuilder carries the
    // grouping methods) and produce the same grouping result.
    let rows: Vec<Vec<DbValue>> = linq!(
        ctx.set::<LqItem>();
        select (i.category, avg(i.amount));
        group_by i.category
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(rows.len(), 2, "one row per category");
}

#[tokio::test]
async fn select_aggregate_with_having() {
    let mut ctx = seed().await;

    let rows: Vec<Vec<DbValue>> = linq!(
        ctx.set::<LqItem>();
        select (i.category, sum(i.amount));
        group_by i.category;
        having sum(i.amount) > 15
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(rows.len(), 1, "hardware sums to 15, not > 15");
    assert_eq!(as_string(&rows[0][0]), "software");
    assert_eq!(as_i64(&rows[0][1]), 20);
}

#[tokio::test]
async fn select_count_without_grouping() {
    let mut ctx = seed().await;

    let rows: Vec<Vec<DbValue>> = linq!(ctx.set::<LqItem>(); select count())
        .to_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(as_i64(&rows[0][0]), 3);

    // `count(col)` counts non-NULL values of that column.
    let rows: Vec<Vec<DbValue>> = linq!(ctx.set::<LqItem>(); select count(i.category))
        .to_list()
        .await
        .unwrap();
    assert_eq!(as_i64(&rows[0][0]), 3);
}

#[tokio::test]
async fn select_then_order_by_and_take() {
    let mut ctx = seed().await;

    let rows: Vec<Vec<DbValue>> = linq!(
        ctx.set::<LqItem>();
        select (i.id, i.name);
        order_by i.id;
        take 2
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(as_i64(&rows[0][0]), 1);
    assert_eq!(as_string(&rows[0][1]), "A");
}

#[tokio::test]
async fn select_min_max_with_group_by() {
    let mut ctx = seed().await;

    let rows: Vec<Vec<DbValue>> = linq!(
        ctx.set::<LqItem>();
        select (i.category, min(i.amount), max(i.amount));
        group_by i.category;
        order_by i.category
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(as_string(&rows[0][0]), "hardware");
    assert_eq!(as_i64(&rows[0][1]), 5, "min hardware amount");
    assert_eq!(as_i64(&rows[0][2]), 10, "max hardware amount");
    assert_eq!(as_string(&rows[1][0]), "software");
    assert_eq!(as_i64(&rows[1][1]), 20);
    assert_eq!(as_i64(&rows[1][2]), 20);
}

#[test]
fn a_plain_column_select_keeps_the_2_6_sql_shape() {
    let mut ctx = make_ctx();
    ctx.set::<LqItem>();

    // 2.6 emitted `SELECT id, name` (no `AS` aliases); the typed projection
    // path must only be used when a select contains aggregate items.
    let sql = linq!(ctx.set::<LqItem>(); select (i.id, i.name)).to_sql();
    assert_eq!(sql, "SELECT id, name FROM lq_items");

    // An aggregate item switches to the typed projection list, which carries
    // deterministic aliases.
    let mut ctx = make_ctx();
    ctx.set::<LqItem>();
    let sql = linq!(ctx.set::<LqItem>(); group_by i.category; select (i.category, sum(i.amount)))
        .to_sql();
    assert_eq!(
        sql,
        "SELECT category AS category, SUM(amount) AS sum_amount FROM lq_items GROUP BY category"
    );
}
