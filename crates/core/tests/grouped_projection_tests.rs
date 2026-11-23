//! EF-01/EF-06: typed grouped projections.
//!
//! Covered here:
//! - SQL shape of `GROUP BY` + aggregate projections + aliases + `ORDER BY`
//!   alias + pagination, multi-key grouping, `COALESCE` / constant column /
//!   `CASE` projections;
//! - parameter order `CTE → projection → WHERE → HAVING → set operation`
//!   (`all_params()` matches the placeholder order, `?` and PostgreSQL `$N`);
//! - SQLite end-to-end: real grouped rows via `select_items` + `to_list_rows`,
//!   alias ordering, `HAVING` filtering, `COALESCE` on an all-`NULL` group and
//!   typed rejection of a SQL `NULL` aggregate without `COALESCE`.

mod common;

use common::PgLikeGenerator;

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::{
    AggKind, BoolExpr, CaseArm, CompareOp, FilterCondition, HavingExpr, QueryBuilder, QueryState,
    SelectItem, SqlExpr,
};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("grp_sales")]
struct Sale {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    category: String,
    #[required]
    region: String,
    /// Nullable integer amount column — SUM stays integer-exact and an
    /// all-`NULL` group exercises the COALESCE path.
    amount: Option<i64>,
    quantity: i32,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("context")
}

/// Seeds 5 sales rows and returns a context whose change tracker is empty.
async fn seeded_ctx() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<Sale>();
    ctx.ensure_created().await.expect("ensure_created");

    for (category, region, amount, quantity) in [
        ("books", "north", 100, 1),
        ("books", "south", 200, 2),
        ("toys", "north", 50, 3),
        ("toys", "south", 150, 4),
        ("games", "north", 1000, 5),
    ] {
        ctx.add::<Sale>(Sale {
            id: 0,
            category: category.to_string(),
            region: region.to_string(),
            amount: Some(amount),
            quantity,
        });
    }
    ctx.save_changes().await.expect("seed save");
    ctx.set::<Sale>().clear_entries();
    ctx
}

// ---------------------------------------------------------------------------
// SQL shape
// ---------------------------------------------------------------------------

/// WHERE category = 'books' AND amount > 10 GROUP BY category ORDER BY alias.
#[test]
fn grouped_projection_sql_shape_has_group_by_alias_order_and_limit() {
    let sql = QueryBuilder::<Sale>::new("grp_sales")
        .group_by(&["category"])
        .select_items(&[
            SelectItem::column("category"),
            SelectItem::sum("amount"),
            SelectItem::count(),
        ])
        .order_by_desc("sum_amount")
        .take(10)
        .to_sql();

    assert_eq!(
        sql,
        "SELECT category AS category, SUM(amount) AS sum_amount, COUNT(*) AS count \
         FROM grp_sales GROUP BY category ORDER BY sum_amount DESC LIMIT 10"
    );
}

/// Multiple group keys + skip/take.
#[test]
fn grouped_projection_sql_shape_supports_multiple_keys_and_pagination() {
    let sql = QueryBuilder::<Sale>::new("grp_sales")
        .group_by(&["category", "region"])
        .select_items(&[
            SelectItem::column("category"),
            SelectItem::column("region"),
            SelectItem::avg("amount"),
        ])
        .order_by("category")
        .skip(2)
        .take(5)
        .to_sql();

    assert_eq!(
        sql,
        "SELECT category AS category, region AS region, AVG(amount) AS avg_amount \
         FROM grp_sales GROUP BY category, region ORDER BY category ASC LIMIT 5 OFFSET 2"
    );
}

/// Clause order does not matter: `select_items` before `group_by` produces the
/// same SQL as the other way round.
#[test]
fn clause_order_select_then_group_matches_group_then_select() {
    let items = [SelectItem::column("category"), SelectItem::sum("amount")];
    let select_first = QueryBuilder::<Sale>::new("grp_sales")
        .select_items(&items)
        .group_by(&["category"])
        .to_sql();
    let group_first = QueryBuilder::<Sale>::new("grp_sales")
        .group_by(&["category"])
        .select_items(&items)
        .to_sql();

    assert_eq!(select_first, group_first);
    assert_eq!(
        select_first,
        "SELECT category AS category, SUM(amount) AS sum_amount \
         FROM grp_sales GROUP BY category"
    );
}

/// DISTINCT applies to the projection list.
#[test]
fn grouped_projection_sql_shape_can_be_distinct() {
    let sql = QueryBuilder::<Sale>::new("grp_sales")
        .group_by(&["category"])
        .select_items(&[SelectItem::column("category")])
        .distinct()
        .to_sql();

    assert_eq!(
        sql,
        "SELECT DISTINCT category AS category FROM grp_sales GROUP BY category"
    );
}

/// COALESCE fallback, constant column and CASE projection in one SELECT list.
#[test]
fn sql_shape_for_coalesce_literal_and_case_projections() {
    let items = vec![
        SelectItem::column("category"),
        SelectItem::sum("amount").coalesce(0i64),
        SelectItem::literal("TENANT", "grant_scope"),
        SelectItem::expr(
            SqlExpr::case(
                vec![CaseArm {
                    condition: FilterCondition::with_values("amount", ">", vec![DbValue::I64(500)]),
                    result: SqlExpr::literal("large"),
                }],
                Some(SqlExpr::literal("small")),
            ),
            "size",
        ),
    ];

    let sql = QueryBuilder::<Sale>::new("grp_sales")
        .select_items(&items)
        .group_by(&["category"])
        .to_sql();

    assert_eq!(
        sql,
        "SELECT category AS category, COALESCE(SUM(amount), ?) AS sum_amount, \
         ? AS grant_scope, CASE WHEN amount > ? THEN ? ELSE ? END AS size \
         FROM grp_sales GROUP BY category"
    );
}

// ---------------------------------------------------------------------------
// Parameter order (CTE → projection → WHERE → HAVING → set operation)
// ---------------------------------------------------------------------------

/// The SELECT-list (`SelectItem`) placeholders come before WHERE/HAVING ones,
/// matching `all_params()` in both `?` and PostgreSQL `$N` dialects.
#[test]
fn params_follow_sql_text_order_projection_where_having() {
    let mut state = QueryState::new("grp_sales");
    state.group_bys = vec!["category".to_string()];
    state.projections = vec![
        SelectItem::literal("TENANT", "grant_scope"),
        SelectItem::sum("amount"),
    ];
    state.where_expr = Some(BoolExpr::Filter(FilterCondition::with_values(
        "quantity",
        ">",
        vec![DbValue::I32(1)],
    )));
    state.parameters.push(DbValue::I32(1));
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Sum,
        col: "amount".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I64(1000),
    });

    let sql = state.to_sql();
    assert_eq!(
        sql,
        "SELECT ? AS grant_scope, SUM(amount) AS sum_amount FROM grp_sales \
         WHERE quantity > ? GROUP BY category HAVING SUM(amount) > ?"
    );
    assert_eq!(
        state.all_params(),
        vec![
            DbValue::String("TENANT".into()),
            DbValue::I32(1),
            DbValue::I64(1000),
        ],
        "projection params, then WHERE, then HAVING"
    );

    // PostgreSQL numbered placeholders must continue in the same order.
    let pg_sql = state.to_sql_with(&PgLikeGenerator);
    assert_eq!(
        pg_sql,
        "SELECT $1 AS grant_scope, SUM(amount) AS sum_amount FROM grp_sales \
         WHERE quantity > $2 GROUP BY category HAVING SUM(amount) > $3"
    );
}

/// Case condition values and result values are collected in projection order.
#[test]
fn case_projection_params_are_ordered_condition_then_result() {
    let item = SelectItem::expr(
        SqlExpr::case(
            vec![CaseArm {
                condition: FilterCondition::with_values("amount", ">", vec![DbValue::I64(100)]),
                result: SqlExpr::literal("big"),
            }],
            Some(SqlExpr::literal("small")),
        ),
        "size",
    );

    let mut state = QueryState::new("grp_sales");
    state.projections = vec![item];

    assert_eq!(
        state.to_sql(),
        "SELECT CASE WHEN amount > ? THEN ? ELSE ? END AS size FROM grp_sales"
    );
    assert_eq!(
        state.all_params(),
        vec![
            DbValue::I64(100),
            DbValue::String("big".into()),
            DbValue::String("small".into()),
        ]
    );
}

// ---------------------------------------------------------------------------
// SQLite end-to-end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn grouped_rows_alias_order_and_having() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(String, i64)> = ctx
        .set::<Sale>()
        .query()
        .group_by(&["category"])
        .select_items(&[SelectItem::column("category"), SelectItem::sum("amount")])
        .having(HavingExpr::Compare {
            agg: AggKind::Sum,
            col: "amount".to_string(),
            op: CompareOp::Gt,
            value: DbValue::I64(250),
        })
        .order_by_desc("sum_amount")
        .to_list_rows::<(String, i64)>()
        .await
        .expect("grouped rows");

    assert_eq!(
        rows,
        vec![("games".to_string(), 1000), ("books".to_string(), 300),],
        "HAVING keeps books(300)/games(1000); alias DESC orders games first"
    );
}

#[tokio::test]
async fn grouped_rows_with_avg_and_count_in_one_projection() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(String, f64, i64)> = ctx
        .set::<Sale>()
        .query()
        .group_by(&["category"])
        .select_items(&[
            SelectItem::column("category"),
            SelectItem::avg("amount"),
            SelectItem::count(),
        ])
        .order_by("category")
        .to_list_rows::<(String, f64, i64)>()
        .await
        .expect("grouped rows with AVG and COUNT");

    assert_eq!(
        rows,
        vec![
            ("books".to_string(), 150.0, 2),
            ("games".to_string(), 1000.0, 1),
            ("toys".to_string(), 100.0, 2),
        ]
    );
}

/// Multi-key grouping returns one row per `(category, region)` pair.
#[tokio::test]
async fn multi_key_grouping_returns_one_row_per_key_pair() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(String, String, i64)> = ctx
        .set::<Sale>()
        .query()
        .group_by(&["category", "region"])
        .select_items(&[
            SelectItem::column("category"),
            SelectItem::column("region"),
            SelectItem::sum("amount"),
        ])
        .order_by("category")
        .order_by("region")
        .to_list_rows::<(String, String, i64)>()
        .await
        .expect("multi-key grouped rows");

    assert_eq!(
        rows,
        vec![
            ("books".to_string(), "north".to_string(), 100),
            ("books".to_string(), "south".to_string(), 200),
            ("games".to_string(), "north".to_string(), 1000),
            ("toys".to_string(), "north".to_string(), 50),
            ("toys".to_string(), "south".to_string(), 150),
        ]
    );
}

/// A group whose `SUM` is SQL `NULL` is rejected by typed decoding unless the
/// projection explicitly requests `COALESCE`.
#[tokio::test]
async fn coalesce_turns_an_all_null_group_into_the_neutral_value() {
    let mut ctx = seeded_ctx().await;
    let mut conn = ctx.provider().get_connection().await.expect("connection");
    conn.execute(
        "INSERT INTO grp_sales (category, region, amount, quantity) \
         VALUES ('misc', 'north', NULL, 9)",
        &[],
    )
    .await
    .expect("insert NULL amount");

    // Without COALESCE the typed terminal sees SQL NULL and reports a typed error.
    let err = ctx
        .set::<Sale>()
        .query()
        .group_by(&["category"])
        .select_items(&[SelectItem::column("category"), SelectItem::sum("amount")])
        .to_list_rows::<(String, i64)>()
        .await
        .expect_err("SUM over an all-NULL group is SQL NULL");
    assert!(
        matches!(err, EFError::TypeConversion(..)),
        "expected TypeConversion, got {err:?}"
    );

    // With the explicit COALESCE the group reports the neutral value.
    let rows: Vec<(String, i64)> = ctx
        .set::<Sale>()
        .query()
        .group_by(&["category"])
        .select_items(&[
            SelectItem::column("category"),
            SelectItem::sum("amount").coalesce(0i64),
        ])
        .order_by("category")
        .to_list_rows::<(String, i64)>()
        .await
        .expect("COALESCE(SUM(amount), 0) handles the NULL group");

    assert_eq!(
        rows,
        vec![
            ("books".to_string(), 300),
            ("games".to_string(), 1000),
            ("misc".to_string(), 0),
            ("toys".to_string(), 200),
        ]
    );
}

/// Constant and CASE projections evaluate in SQLite; the literal is bound as a
/// parameter, so it round-trips as a string.
#[tokio::test]
async fn literal_and_case_projections_execute() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(String, String, String)> = ctx
        .set::<Sale>()
        .query()
        .group_by(&["category"])
        .select_items(&[
            SelectItem::column("category"),
            SelectItem::expr(
                SqlExpr::case(
                    vec![CaseArm {
                        condition: FilterCondition::with_values(
                            "category",
                            "=",
                            vec![DbValue::String("books".to_string())],
                        ),
                        result: SqlExpr::literal("has-books"),
                    }],
                    Some(SqlExpr::literal("other")),
                ),
                "flag",
            ),
            SelectItem::literal("TENANT", "grant_scope"),
        ])
        .order_by("category")
        .to_list_rows::<(String, String, String)>()
        .await
        .expect("literal/CASE projections");

    assert_eq!(
        rows,
        vec![
            (
                "books".to_string(),
                "has-books".to_string(),
                "TENANT".to_string()
            ),
            (
                "games".to_string(),
                "other".to_string(),
                "TENANT".to_string()
            ),
            (
                "toys".to_string(),
                "other".to_string(),
                "TENANT".to_string()
            ),
        ]
    );
}

/// A projection without GROUP BY returns one row for the whole table.
#[tokio::test]
async fn ungrouped_aggregate_projection_returns_one_row() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(i64, i64)> = ctx
        .set::<Sale>()
        .query()
        .select_items(&[SelectItem::sum("amount"), SelectItem::count()])
        .to_list_rows::<(i64, i64)>()
        .await
        .expect("whole-table aggregate projection");

    assert_eq!(rows, vec![(1500, 5)]);
}

/// End-to-end parameter alignment: projection (COALESCE fallback + constant)
/// parameters come first, then WHERE, then HAVING — the same order
/// `all_params()` reports for SQL text order.
#[tokio::test]
async fn projection_where_and_having_params_align_in_sqlite() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(String, i64, String)> = ctx
        .set::<Sale>()
        .query()
        .filter(BoolExpr::Filter(FilterCondition::with_values(
            "quantity",
            ">",
            vec![DbValue::I32(0)],
        )))
        .group_by(&["category"])
        .select_items(&[
            SelectItem::column("category"),
            SelectItem::sum("amount").coalesce(0i64),
            SelectItem::literal("TENANT", "grant_scope"),
        ])
        .having(HavingExpr::Compare {
            agg: AggKind::Sum,
            col: "amount".to_string(),
            op: CompareOp::Gt,
            value: DbValue::I64(250),
        })
        .order_by("category")
        .to_list_rows::<(String, i64, String)>()
        .await
        .expect("projection literal + WHERE + HAVING must bind params in SQL order");

    assert_eq!(
        rows,
        vec![
            ("books".to_string(), 300, "TENANT".to_string()),
            ("games".to_string(), 1000, "TENANT".to_string()),
        ],
        "COALESCE fallback (projection), quantity > 0 (WHERE) and SUM > 250 (HAVING) \
         must line up with their placeholders"
    );
}

#[tokio::test]
async fn entity_materialization_rejects_grouped_queries() {
    let mut ctx = seeded_ctx().await;

    let error = linq!(ctx.set::<Sale>(); group_by s.category)
        .to_list()
        .await
        .expect_err("grouped entity materialization must be rejected");
    assert!(
        matches!(error, EFError::Configuration(..)),
        "expected a configuration error, got {error:?}"
    );

    // A projection query is the supported path.
    let rows: Vec<(String, i64)> = linq!(ctx.set::<Sale>();
        group_by s.category;
        select (s.category, count())
    )
    .to_list_typed_2()
    .await
    .expect("projection rows");
    assert_eq!(rows.len(), 3);
}

#[test]
fn plain_columns_and_typed_items_compose_instead_of_dropping_columns() {
    let mut ctx = make_ctx();
    ctx.set::<Sale>();

    let sql = ctx
        .set::<Sale>()
        .query()
        .select(&[Sale::COLUMN_CATEGORY])
        .select_items(&[SelectItem::sum(Sale::COLUMN_AMOUNT)])
        .group_by(&[Sale::COLUMN_CATEGORY])
        .to_sql();

    assert!(
        sql.contains("category AS category, SUM(amount) AS sum_amount"),
        "the plain select column must be kept: {sql}"
    );
    assert_eq!(
        ctx.set::<Sale>()
            .query()
            .select_items(&[SelectItem::sum(Sale::COLUMN_AMOUNT)])
            .state()
            .projections
            .len(),
        1,
        "state() exposes the typed projections"
    );
}
