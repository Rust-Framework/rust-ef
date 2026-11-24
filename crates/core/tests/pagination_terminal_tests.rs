//! `QueryBuilder::to_page(PagedRequest) -> PagedResult<T>` 鈥?offset pagination
//! terminal.
//!
//! Covers the SQLite end-to-end contract (count shares the filter chain; page
//! 1/N; out-of-range page; empty table; caller ORDER BY/skip/take must not leak
//! into the count; DTO mapping; non-`Clone` entities) and the per-dialect SQL
//! shape of the two statements `to_page` issues.

use rust_core::{PagedRequest, PagedResult};
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::error::EFError;
use rust_ef::prelude::*;
use rust_ef::query::QueryState;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("pagination_page_orders")]
struct PageOrder {
    #[primary_key]
    #[auto_increment]
    id: i32,
    tenant_id: i32,
    created_at: i32,
    amount: i64,
    #[max_length(16)]
    status: String,
}

/// Deliberately does **not** derive `Clone`: `to_page` must not require
/// `T: Clone` (the builder's `Clone` impl is manual for that reason).
#[derive(Debug, EntityType)]
#[table("pagination_non_clone_items")]
struct NonCloneItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[max_length(32)]
    name: String,
}

#[derive(Debug, PartialEq)]
struct OrderDto {
    id: i32,
    amount: i64,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("ctx")
}

fn request(page: u32, page_size: u32) -> PagedRequest {
    PagedRequest { page, page_size }
}

/// Seeds `count` rows for `tenant_id`, with strictly increasing `created_at`.
async fn seed_orders(ctx: &mut DbContext, tenant_id: i32, count: i32, tenant_offset: i32) {
    for i in 0..count {
        ctx.add::<PageOrder>(PageOrder {
            id: 0,
            tenant_id,
            created_at: tenant_offset + i,
            amount: i64::from(i) * 10,
            status: "paid".into(),
        });
    }
    ctx.save_changes().await.expect("seed orders");
}

fn ordered_query(ctx: &mut DbContext, tenant_id: i32) -> rust_ef::query::QueryBuilder<PageOrder> {
    ctx.set::<PageOrder>()
        .query()
        .filter(linq!(|o: PageOrder| o.tenant_id == tenant_id))
        .order_by(PageOrder::COLUMN_CREATED_AT)
        .order_by(PageOrder::COLUMN_ID) // unique tie-breaker for a stable page
}

fn created_ats(page: &PagedResult<PageOrder>) -> Vec<i32> {
    page.items.iter().map(|order| order.created_at).collect()
}

// ---------------------------------------------------------------------------
// SQLite end to end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn count_and_page_share_the_filter_chain() {
    let mut ctx = make_ctx();
    ctx.set::<PageOrder>();
    ctx.ensure_created().await.expect("ensure_created");
    seed_orders(&mut ctx, 1, 25, 1).await;
    seed_orders(&mut ctx, 2, 3, 100).await;

    let page1 = ordered_query(&mut ctx, 1)
        .to_page(&request(1, 10))
        .await
        .expect("page 1");
    assert_eq!(page1.total, 25, "total counts only the filtered tenant");
    assert_eq!(created_ats(&page1), (1..=10).collect::<Vec<_>>());

    let page2 = ordered_query(&mut ctx, 1)
        .to_page(&request(2, 10))
        .await
        .expect("page 2");
    assert_eq!(page2.total, 25);
    assert_eq!(created_ats(&page2), (11..=20).collect::<Vec<_>>());

    let page3 = ordered_query(&mut ctx, 1)
        .to_page(&request(3, 10))
        .await
        .expect("page 3");
    assert_eq!(page3.total, 25);
    assert_eq!(created_ats(&page3), (21..=25).collect::<Vec<_>>());

    // No row of tenant 2 leaked into the filtered page.
    assert!(page1
        .items
        .iter()
        .chain(page2.items.iter())
        .chain(page3.items.iter())
        .all(|order| order.tenant_id == 1));
}

/// The key regression: `count()` rejects `ORDER BY` / `skip` / `take`, so
/// `to_page` must strip them from its counter clone while keeping the page's own
/// ordering and the caller's pre-paging chain harmless.
#[tokio::test]
async fn count_ignores_order_by_and_caller_paging() {
    let mut ctx = make_ctx();
    ctx.set::<PageOrder>();
    ctx.ensure_created().await.expect("ensure_created");
    seed_orders(&mut ctx, 1, 25, 1).await;

    let page = ctx
        .set::<PageOrder>()
        .query()
        .filter(linq!(|o: PageOrder| o.tenant_id == 1))
        .order_by_desc(PageOrder::COLUMN_CREATED_AT)
        // A caller-paging chain that a COUNT(*) terminal rejects; `to_page`
        // replaces it with the requested page.
        .skip(999)
        .take(3)
        .to_page(&request(2, 10))
        .await
        .expect("to_page must clear ORDER BY/skip/take for the count");

    assert_eq!(page.total, 25, "count must not inherit caller paging/order");
    assert_eq!(page.items.len(), 10, "the requested page size wins");
    assert_eq!(
        created_ats(&page),
        (6..=15).rev().collect::<Vec<_>>(),
        "descending ORDER BY applies to the page only"
    );
}

#[tokio::test]
async fn page_past_the_end_returns_no_items_and_the_real_total() {
    let mut ctx = make_ctx();
    ctx.set::<PageOrder>();
    ctx.ensure_created().await.expect("ensure_created");
    seed_orders(&mut ctx, 1, 25, 1).await;

    let beyond = ordered_query(&mut ctx, 1)
        .to_page(&request(99, 10))
        .await
        .expect("out-of-range page is not an error");
    assert!(beyond.items.is_empty());
    assert_eq!(beyond.total, 25);

    // Huge page number: offset is a u64 that cannot be truncated to usize.
    let huge = ordered_query(&mut ctx, 1)
        .to_page(&request(u32::MAX, 100))
        .await
        .expect("huge page number");
    assert!(huge.items.is_empty());
    assert_eq!(huge.total, 25);
}

#[tokio::test]
async fn empty_table_has_zero_total_and_no_items() {
    let mut ctx = make_ctx();
    ctx.set::<PageOrder>();
    ctx.ensure_created().await.expect("ensure_created");

    let page = ordered_query(&mut ctx, 1)
        .to_page(&request(1, 10))
        .await
        .expect("empty page");
    assert_eq!(page.total, 0);
    assert!(page.items.is_empty());
}

#[tokio::test]
async fn page_size_boundaries_return_the_requested_row_counts() {
    let mut ctx = make_ctx();
    ctx.set::<PageOrder>();
    ctx.ensure_created().await.expect("ensure_created");
    seed_orders(&mut ctx, 1, 25, 1).await;

    let one = ordered_query(&mut ctx, 1)
        .to_page(&request(1, 1))
        .await
        .expect("page_size 1");
    assert_eq!(one.items.len(), 1);
    assert_eq!(one.total, 25);

    let max = ordered_query(&mut ctx, 1)
        .to_page(&request(1, rust_core::MAX_PAGE_SIZE))
        .await
        .expect("page_size MAX_PAGE_SIZE");
    assert_eq!(max.items.len(), 25);
    assert_eq!(max.total, 25);
}

#[tokio::test]
async fn entity_page_maps_to_a_dto_page_with_the_same_total() {
    let mut ctx = make_ctx();
    ctx.set::<PageOrder>();
    ctx.ensure_created().await.expect("ensure_created");
    seed_orders(&mut ctx, 1, 3, 1).await;

    let page = ordered_query(&mut ctx, 1)
        .to_page(&request(1, 2))
        .await
        .expect("entity page");
    assert_eq!(page.total, 3);

    let dto_page: PagedResult<OrderDto> = page.map_items(|order| OrderDto {
        id: order.id,
        amount: order.amount,
    });
    assert_eq!(dto_page.total, 3, "map_items preserves total");
    assert_eq!(
        dto_page.items,
        vec![
            OrderDto { id: 1, amount: 0 },
            OrderDto { id: 2, amount: 10 }
        ]
    );
}

#[tokio::test]
async fn to_page_does_not_require_a_clone_entity() {
    let mut ctx = make_ctx();
    ctx.set::<NonCloneItem>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx.add::<NonCloneItem>(NonCloneItem {
        id: 0,
        name: "only".into(),
    });
    ctx.save_changes().await.expect("seed");

    let query = ctx.set::<NonCloneItem>().query();
    // `QueryBuilder::clone` must not require `T: Clone` (checked structurally
    // here; the manual impl is the only way this line compiles).
    let _fork = query.clone();

    let page = query
        .to_page(&request(1, 10))
        .await
        .expect("non-Clone entity page");
    assert_eq!(page.total, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].name, "only");
}

/// Shapes whose total is not well defined keep `count()`'s typed error instead
/// of silently reporting a different number than the page.
#[tokio::test]
async fn unsupported_shapes_report_a_typed_error() {
    let mut ctx = make_ctx();
    ctx.set::<PageOrder>();
    ctx.ensure_created().await.expect("ensure_created");

    let error = ordered_query(&mut ctx, 1)
        .distinct()
        .to_page(&request(1, 10))
        .await
        .expect_err("DISTINCT + COUNT(*) is not a defined total");
    match error {
        EFError::Configuration(message, _) => {
            assert!(
                message.contains("DISTINCT"),
                "expected a DISTINCT explanation, got: {message}"
            );
        }
        other => panic!("expected EFError::Configuration, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Per-dialect SQL shape (the two statements to_page issues)
// ---------------------------------------------------------------------------

/// `to_page`'s counter clone keeps FROM/WHERE and drops ORDER BY/skip/take, so
/// the COUNT statement is exactly this shape on every dialect.
#[test]
fn count_sql_shape_is_dialect_agnostic() {
    let mut state = QueryState::new("pagination_page_orders");
    state.where_expr = Some(linq!(|o: PageOrder| o.tenant_id == 1).into_expr());
    state.set_count();

    assert_eq!(
        state.to_sql_with(&rust_ef_sqlite::SqliteSqlGenerator::new()),
        "SELECT COUNT(*) FROM pagination_page_orders WHERE tenant_id = ?"
    );
    assert_eq!(
        state.to_sql_with(&rust_ef_postgres::PostgresSqlGenerator::new()),
        "SELECT COUNT(*) FROM pagination_page_orders WHERE tenant_id = $1"
    );
    assert_eq!(
        state.to_sql_with(&rust_ef_mysql::MySqlSqlGenerator::new()),
        "SELECT COUNT(*) FROM pagination_page_orders WHERE tenant_id = ?"
    );
}

/// The page statement keeps ORDER BY and uses the dialect's LIMIT/OFFSET order.
#[test]
fn page_sql_shape_uses_each_dialect_pagination() {
    let mut state = QueryState::new("pagination_page_orders");
    state.where_expr = Some(linq!(|o: PageOrder| o.tenant_id == 1).into_expr());
    state.orderings.push(rust_ef::query::OrderBy::new(
        "created_at",
        rust_ef::query::OrderDirection::Descending,
    ));
    state.offset = Some(10);
    state.limit = Some(10);

    assert_eq!(
        state.to_sql_with(&rust_ef_sqlite::SqliteSqlGenerator::new()),
        "SELECT * FROM pagination_page_orders WHERE tenant_id = ? ORDER BY created_at DESC \
         LIMIT 10 OFFSET 10"
    );
    assert_eq!(
        state.to_sql_with(&rust_ef_postgres::PostgresSqlGenerator::new()),
        "SELECT * FROM pagination_page_orders WHERE tenant_id = $1 ORDER BY created_at DESC \
         OFFSET 10 LIMIT 10"
    );
    assert_eq!(
        state.to_sql_with(&rust_ef_mysql::MySqlSqlGenerator::new()),
        "SELECT * FROM pagination_page_orders WHERE tenant_id = ? ORDER BY created_at DESC \
         LIMIT 10 OFFSET 10"
    );
}

/// Why `to_page` clears first: the same COUNT state with ORDER BY or paging is
/// rejected by `QueryState::validate` instead of emitting `COUNT(*) ... ORDER
/// BY` (undefined total / wasted work).
#[test]
fn count_with_order_by_or_paging_is_rejected() {
    let mut count_state = QueryState::new("pagination_page_orders");
    count_state.set_count();

    let sqlite = rust_ef_sqlite::SqliteSqlGenerator::new();
    assert!(count_state.clone().try_compile_with(&sqlite).is_ok());

    let mut ordered = count_state.clone();
    ordered.orderings.push(rust_ef::query::OrderBy::new(
        "created_at",
        rust_ef::query::OrderDirection::Ascending,
    ));
    assert!(
        ordered.try_compile_with(&sqlite).is_err(),
        "COUNT(*) + ORDER BY must be a typed error, not valid SQL"
    );

    let mut paged = count_state;
    paged.limit = Some(10);
    assert!(
        paged.try_compile_with(&sqlite).is_err(),
        "COUNT(*) + LIMIT must be a typed error, not valid SQL"
    );
}
