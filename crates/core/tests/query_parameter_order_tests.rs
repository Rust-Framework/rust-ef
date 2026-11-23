//! Parameter-order regressions for the 2.7 projection / HAVING / update paths.
//!
//! The SQL-dependent invariants are:
//!
//! * `all_params()` follows the SQL text order:
//!   CTE → SELECT projections → WHERE → HAVING → set operations;
//! * a builder call order that differs from the SQL order (e.g. HAVING added
//!   before the WHERE filter) must not shift the bound values;
//! * set-based `UPDATE` binds its SET expressions before the WHERE parameters.

mod common;

use common::PgLikeGenerator;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::query::{AggKind, CompareOp, HavingExpr, SelectItem};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("po_sales")]
struct PoSale {
    #[primary_key]
    #[auto_increment]
    id: i32,
    amount: i64,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

async fn seeded_ctx() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<PoSale>();
    ctx.ensure_created().await.unwrap();
    for (id, amount) in [(1, 150), (2, 2000), (3, 50)] {
        ctx.add::<PoSale>(PoSale { id, amount });
    }
    ctx.save_changes().await.unwrap();
    ctx.clear_tracking();
    ctx
}

#[tokio::test]
async fn having_added_before_the_filter_keeps_sql_parameter_order() {
    let mut ctx = make_ctx();
    ctx.set::<PoSale>();

    let having = HavingExpr::Compare {
        agg: AggKind::Count,
        col: "id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I64(1),
    };
    // Deliberately registers HAVING before the WHERE filter: the generated SQL
    // still emits WHERE first, so the bound vector must follow the SQL order.
    let query = ctx
        .set::<PoSale>()
        .query()
        .having_expr_internal(having)
        .filter(linq!(|s: PoSale| s.amount > 10));

    let (sql, params) = query.compile_sql();
    assert_eq!(
        sql,
        "SELECT * FROM po_sales WHERE amount > ? HAVING COUNT(id) > ?"
    );
    assert_eq!(params, vec![DbValue::I32(10), DbValue::I64(1)]);

    // The PostgreSQL placeholder numbering follows the same order.
    let pg_sql = query.state().to_sql_with(&PgLikeGenerator);
    assert_eq!(
        pg_sql,
        "SELECT * FROM po_sales WHERE amount > $1 HAVING COUNT(id) > $2"
    );
}

#[tokio::test]
async fn cte_plus_projection_parameters_follow_sql_text_order() {
    let mut ctx = make_ctx();
    ctx.set::<PoSale>();

    let cte_filter = linq!(|s: PoSale| s.amount > 100).into_expr();
    let query = ctx
        .set::<PoSale>()
        .query()
        .with_cte_typed("rich", PoSale::TABLE, cte_filter)
        .from_cte("rich")
        .filter(linq!(|s: PoSale| s.amount < 999));

    // Projection parameters are not reachable from the builder's public state
    // API, so clone the accumulated state and append the projection items
    // exactly like `select_items` does.
    let mut state = query.state().clone();
    state.projections.push(SelectItem::literal(
        DbValue::String("REPORT".to_string()),
        "grant_scope",
    ));
    state
        .projections
        .push(SelectItem::column(PoSale::COLUMN_ID));

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert_eq!(
        sql,
        "WITH rich AS (SELECT * FROM \"po_sales\" WHERE amount > $1) \
         SELECT $2 AS grant_scope, id AS id FROM rich WHERE amount < $3"
    );
    assert_eq!(
        state.all_params(),
        vec![
            DbValue::I32(100),
            DbValue::String("REPORT".to_string()),
            DbValue::I32(999),
        ]
    );
}

#[tokio::test]
async fn cte_plus_projection_runs_with_the_expected_rows() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(String, i32)> = ctx
        .set::<PoSale>()
        .query()
        .with_cte_typed(
            "rich",
            PoSale::TABLE,
            linq!(|s: PoSale| s.amount > 100).into_expr(),
        )
        .from_cte("rich")
        .filter(linq!(|s: PoSale| s.amount < 999))
        .select_items(&[
            SelectItem::literal(DbValue::String("REPORT".to_string()), "grant_scope"),
            SelectItem::column(PoSale::COLUMN_ID),
        ])
        .to_list_typed_2()
        .await
        .unwrap();

    // amount 150 goes through both bindings; if the CTE and outer WHERE values
    // were swapped the result would be empty (or a different row).
    assert_eq!(rows, vec![("REPORT".to_string(), 1)]);
}

#[tokio::test]
async fn expression_updates_bind_set_values_before_the_where_clause() {
    let mut ctx = make_ctx();
    ctx.set::<PoSale>();

    let update = ctx
        .set::<PoSale>()
        .query()
        .filter(linq!(|s: PoSale| s.id == 7))
        .execute_update()
        .set_column(PoSale::COLUMN_AMOUNT, DbValue::I64(100))
        .increment(PoSale::COLUMN_AMOUNT, 5);

    assert_eq!(
        update.to_sql(),
        "UPDATE po_sales SET amount = ?, amount = amount + ? WHERE id = ?"
    );
    assert_eq!(
        update.params(),
        vec![DbValue::I64(100), DbValue::I32(5), DbValue::I32(7)]
    );
}
