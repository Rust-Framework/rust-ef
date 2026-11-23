//! Query validation and portable raw-fragment compilation (rust-ef 2.8).
//!
//! These tests pin the "invalid query is a typed error, not invalid SQL"
//! contract added by the single-pass compiler/`SelectList` refactor.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::error::EFError;
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::{
    AggKind, BoolExpr, CompareOp, JoinKind, JoinSpec, QueryBuilder, QueryState, SelectItem,
    SelectList,
};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

mod common;

use common::PgLikeGenerator;

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, EntityType)]
#[table("qv_rows")]
struct QvRow {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    pub amount: i64,

    #[required]
    #[max_length(32)]
    pub label: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("qv_other")]
struct QvOther {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    pub amount: i64,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("context")
}

async fn seeded_ctx() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<QvRow>();
    ctx.set::<QvOther>();
    ctx.ensure_created().await.expect("schema");
    for (amount, label) in [(10_i64, "a"), (20, "a"), (5, "b")] {
        ctx.add(QvRow {
            id: 0,
            amount,
            label: label.to_string(),
        });
    }
    ctx.save_changes().await.expect("seed");
    ctx
}

// ---------------------------------------------------------------------------
// Invalid states must be typed errors.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn count_with_distinct_is_a_typed_configuration_error() {
    let mut ctx = seeded_ctx().await;
    let error = linq!(ctx.set::<QvRow>(); distinct).count().await;
    assert!(
        matches!(error, Err(EFError::Configuration(..))),
        "COUNT(DISTINCT *) must be rejected: {error:?}"
    );

    // The supported spelling: project COUNT(DISTINCT column).
    let counts: Vec<i64> = ctx
        .set::<QvRow>()
        .query()
        .select_items(&[SelectItem::count_distinct(QvRow::COLUMN_LABEL)])
        .to_list_typed_1()
        .await
        .expect("count distinct");
    assert_eq!(counts, vec![2]);
}

#[test]
fn star_select_with_group_by_is_rejected_by_validation() {
    let mut state = QueryState::new("qv_rows");
    state.group_bys = vec!["label".to_string()];
    let error = state
        .try_compile_with(&PgLikeGenerator)
        .expect_err("SELECT * + GROUP BY is not executable");
    assert!(
        matches!(error, EFError::Configuration(..)),
        "expected configuration error, got {error:?}"
    );
    // `to_sql` remains usable for shape/diagnostics (unchecked).
    assert!(state.to_sql().contains("GROUP BY label"));
}

#[test]
fn from_after_join_is_rejected() {
    let mut state = QueryState::new("qv_rows");
    state
        .joins
        .push(JoinSpec::new(JoinKind::Inner, "qv_other", Vec::new()));
    state.mark_from_changed_after_join();
    let error = state
        .try_compile_with(&PgLikeGenerator)
        .expect_err("from after join must be rejected");
    assert!(
        matches!(error, EFError::Configuration(..)),
        "expected configuration error, got {error:?}"
    );
}

#[test]
fn duplicate_projection_aliases_are_rejected() {
    let mut state = QueryState::new("qv_rows");
    state.set_projections(vec![
        SelectItem::column("label"),
        SelectItem::column("other.label"),
    ]);
    let error = state
        .try_compile_with(&PgLikeGenerator)
        .expect_err("duplicate alias must be rejected");
    assert!(
        matches!(error, EFError::Configuration(..)),
        "expected configuration error, got {error:?}"
    );

    // An explicit alias disambiguates.
    state.set_projections(vec![
        SelectItem::column("label"),
        SelectItem::column("other.label").alias("other_label"),
    ]);
    assert!(state.try_compile_with(&PgLikeGenerator).is_ok());
}

#[test]
fn select_list_is_a_single_enum_shape() {
    let state = QueryState::new("qv_rows");
    assert!(matches!(state.select_list(), SelectList::Star));
    assert!(!state.is_count());

    let mut state = QueryState::new("qv_rows");
    state.set_count();
    assert!(state.is_count());
    assert!(state.select_list().is_count());
}

// ---------------------------------------------------------------------------
// Portable raw fragments on PostgreSQL (raw CTE + set-operation operands).
// ---------------------------------------------------------------------------

#[test]
fn raw_cte_question_placeholders_are_ported_to_the_provider() {
    let query = QueryBuilder::<QvRow>::new("qv_rows")
        .with_cte_internal(
            "big",
            "SELECT * FROM qv_rows WHERE amount > ?",
            vec![DbValue::I64(10)],
            &[],
        )
        .from_cte("big")
        .filter(linq!(|r: QvRow| r.label == "a"));

    let (sql, params) = query
        .state()
        .compile_sql_with::<QvRow>(&PgLikeGenerator)
        .expect("compile");
    assert_eq!(
        sql,
        "WITH \"big\" AS (SELECT * FROM qv_rows WHERE amount > $1) \
         SELECT * FROM big WHERE label = $2"
    );
    assert_eq!(
        params,
        vec![DbValue::I64(10), DbValue::String("a".to_string()),]
    );
}

#[tokio::test]
async fn set_operation_operand_question_placeholders_are_ported() {
    let mut ctx = seeded_ctx().await;

    let operand = ctx
        .set::<QvRow>()
        .query()
        .filter(linq!(|r: QvRow| r.amount > 100))
        .compile_sql()
        .expect("operand compile");
    // The portable `?` operand (SQLite generator) is shifted and ported when the
    // main query compiles for PostgreSQL.
    let query = ctx
        .set::<QvRow>()
        .query()
        .filter(linq!(|r: QvRow| r.amount < 999))
        .union_internal(operand);
    let (sql, params) = query
        .state()
        .compile_sql_with::<QvRow>(&PgLikeGenerator)
        .expect("compile");
    assert!(
        sql.contains("WHERE amount < $1 UNION SELECT * FROM qv_rows WHERE amount > $2"),
        "unexpected SQL: {sql}"
    );
    assert_eq!(params, vec![DbValue::I32(999), DbValue::I32(100)]);
}

// ---------------------------------------------------------------------------
// HAVING is lowered into BoolExpr and shares the WHERE compiler.
// ---------------------------------------------------------------------------

#[test]
fn having_lowers_into_the_shared_boolean_algebra() {
    let having = BoolExpr::aggregate_compare(
        AggKind::Sum,
        "amount".to_string(),
        CompareOp::Gt,
        DbValue::I64(5),
    );
    assert!(matches!(having, BoolExpr::Compare { .. }));

    let mut state = QueryState::new("qv_rows");
    state.group_bys = vec!["label".to_string()];
    state.set_projections(vec![SelectItem::column("label"), SelectItem::sum("amount")]);
    state.havings.push(BoolExpr::aggregate_compare(
        AggKind::Sum,
        "amount".to_string(),
        CompareOp::Gt,
        DbValue::I64(5),
    ));
    let (sql, params) = state
        .compile_sql_with::<QvRow>(&PgLikeGenerator)
        .expect("compile");
    assert!(
        sql.contains("HAVING SUM(amount) > $1"),
        "unexpected SQL: {sql}"
    );
    assert_eq!(params, vec![DbValue::I64(5)]);
}

// ---------------------------------------------------------------------------
// Typed projection terminals are strict about row width.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn typed_terminals_reject_a_mismatched_projection_width() {
    let mut ctx = seeded_ctx().await;
    let error = ctx
        .set::<QvRow>()
        .query()
        .select(&[QvRow::COLUMN_ID, QvRow::COLUMN_AMOUNT, QvRow::COLUMN_LABEL])
        .to_list_typed_2::<i32, i64>()
        .await
        .expect_err("3 columns cannot be read as a 2-tuple");
    assert!(
        matches!(error, EFError::TypeConversion(..)),
        "expected a TypeConversion error, got {error:?}"
    );
}

#[tokio::test]
async fn aggregate_terminal_with_group_by_is_rejected() {
    let mut ctx = seeded_ctx().await;
    let error = ctx
        .set::<QvRow>()
        .query()
        .group_by(&[QvRow::COLUMN_LABEL])
        .sum_f64(QvRow::COLUMN_AMOUNT)
        .await;
    assert!(
        matches!(error, Err(EFError::Configuration(..))),
        "an aggregate terminal with GROUP BY must be rejected: {error:?}"
    );
}

#[test]
fn multiple_having_fragments_keep_their_boolean_grouping() {
    // Regression: `a AND (b OR c)` must not compile to `a AND (b) OR (c)`.
    let mut state = QueryState::new("qv_rows");
    state.group_bys = vec!["label".to_string()];
    state.set_projections(vec![SelectItem::column("label"), SelectItem::count()]);
    state.havings.push(BoolExpr::aggregate_compare(
        AggKind::Sum,
        "amount".to_string(),
        CompareOp::Gt,
        DbValue::I64(1000),
    ));
    state.havings.push(BoolExpr::Or(
        Box::new(BoolExpr::aggregate_compare(
            AggKind::Sum,
            "amount".to_string(),
            CompareOp::Gt,
            DbValue::I64(1),
        )),
        Box::new(BoolExpr::aggregate_compare(
            AggKind::Max,
            "amount".to_string(),
            CompareOp::Lt,
            DbValue::I64(0),
        )),
    ));

    let sql = state.to_sql();
    assert!(
        sql.contains("HAVING (SUM(amount) > ?) AND ((SUM(amount) > ?) OR (MAX(amount) < ?))"),
        "HAVING fragments must be parenthesised: {sql}"
    );
    assert_eq!(state.all_params().len(), 3);
}

#[tokio::test]
async fn multiple_having_fragments_are_executed_with_correct_precedence() {
    let mut ctx = seeded_ctx().await;
    let rows: Vec<(String, i64)> = ctx
        .set::<QvRow>()
        .query()
        .select_items(&[SelectItem::column(QvRow::COLUMN_LABEL), SelectItem::count()])
        .group_by(&[QvRow::COLUMN_LABEL])
        // `SUM > 1000` is false for every group, so the whole AND must be false
        // even though the OR branch is true for both groups.
        .having(BoolExpr::aggregate_compare(
            AggKind::Sum,
            QvRow::COLUMN_AMOUNT.to_string(),
            CompareOp::Gt,
            DbValue::I64(1000),
        ))
        .having(BoolExpr::Or(
            Box::new(BoolExpr::aggregate_compare(
                AggKind::Sum,
                QvRow::COLUMN_AMOUNT.to_string(),
                CompareOp::Gt,
                DbValue::I64(0),
            )),
            Box::new(BoolExpr::aggregate_compare(
                AggKind::Max,
                QvRow::COLUMN_AMOUNT.to_string(),
                CompareOp::Lt,
                DbValue::I64(0),
            )),
        ))
        .to_list_typed_2()
        .await
        .expect("grouped rows");
    assert!(
        rows.is_empty(),
        "the first AND operand fails for every group: {rows:?}"
    );
}

#[test]
fn execute_update_rejects_a_deferred_query_validation_error() {
    let mut ctx = make_ctx();
    ctx.set::<QvRow>();
    ctx.set::<QvOther>();

    // `from_cte` after `join` records a deferred validation error; the read
    // path rejects it, and `execute_update` must reject it too (it must not
    // silently run an UPDATE with the filter dropped).
    let result = ctx
        .set::<QvRow>()
        .query()
        .join::<QvOther>(JoinKind::Inner, &[("id", "id")])
        .from_cte("blocked")
        .execute_update()
        .set_column(QvRow::COLUMN_LABEL, "x")
        .compile_sql();
    assert!(
        matches!(result, Err(EFError::Configuration(..))),
        "execute_update must reject a deferred validation error: {result:?}"
    );
}

#[test]
fn exists_subquery_predicate_keeps_its_or_grouping() {
    use rust_ef::query::{BoolExpr, FilterCondition, SubquerySpec};

    let spec = SubquerySpec {
        navigation_field: "posts".to_string(),
        related_type_name: "Post".to_string(),
        predicate: Some(Box::new(BoolExpr::Or(
            Box::new(BoolExpr::Filter(FilterCondition::with_values(
                "a",
                "=",
                vec![DbValue::I64(1)],
            ))),
            Box::new(BoolExpr::Filter(FilterCondition::with_values(
                "b",
                "=",
                vec![DbValue::I64(2)],
            ))),
        ))),
        outer_table: "blogs".to_string(),
        related_table: "posts".to_string(),
        fk_column: "blog_id".to_string(),
        outer_pk_column: "id".to_string(),
    };
    let mut state = QueryState::new("blogs");
    state.where_expr = Some(BoolExpr::Exists(Box::new(spec)));

    let sql = state.to_sql();
    assert!(
        sql.contains("AND ((a = ?) OR (b = ?))"),
        "the EXISTS predicate must stay parenthesised under AND: {sql}"
    );
}

#[test]
fn raw_fragments_validate_placeholder_arity() {
    // Raw CTE with two `?` but one bound value: typed error, not a driver error.
    let query = QueryBuilder::<QvRow>::new("qv_rows")
        .with_cte_internal(
            "bad",
            "SELECT * FROM qv_rows WHERE a = ? AND b = ?",
            vec![DbValue::I64(1)],
            &[],
        )
        .from_cte("bad");
    let error = query
        .state()
        .try_compile_with(&PgLikeGenerator)
        .expect_err("placeholder/value mismatch must be rejected");
    assert!(
        matches!(error, EFError::Configuration(..)),
        "expected configuration error, got {error:?}"
    );
}

#[test]
fn set_operations_reject_single_value_aggregate_shapes() {
    let operand = {
        let mut state = QueryState::new("qv_rows");
        state.set_projections(vec![SelectItem::column("id")]);
        state.to_sql()
    };
    let mut state = QueryState::new("qv_rows");
    state.set_count();
    state.set_operations.push(rust_ef::query::SetOpSpec {
        operator: rust_ef::query::SetOperator::Union,
        operand_sql: operand,
        operand_params: Vec::new(),
    });
    let error = state
        .try_compile_with(&PgLikeGenerator)
        .expect_err("COUNT + UNION must be rejected");
    assert!(matches!(error, EFError::Configuration(..)));
}

#[test]
fn ordering_and_pagination_after_a_set_operation_apply_to_the_combined_result() {
    let operand = {
        let mut state = QueryState::new("qv_rows");
        state.set_projections(vec![SelectItem::column("id")]);
        state.to_sql()
    };
    let mut state = QueryState::new("qv_rows");
    state.set_projections(vec![SelectItem::column("id")]);
    state.set_operations.push(rust_ef::query::SetOpSpec {
        operator: rust_ef::query::SetOperator::Union,
        operand_sql: operand,
        operand_params: Vec::new(),
    });
    state.orderings.push(rust_ef::query::OrderBy::new(
        "id",
        rust_ef::query::OrderDirection::Descending,
    ));
    state.limit = Some(5);

    let sql = state.to_sql();
    let union = sql.find("UNION").expect("union present");
    let order = sql.find("ORDER BY").expect("order by present");
    let limit = sql.find("LIMIT").expect("limit present");
    assert!(union < order && order < limit, "unexpected SQL: {sql}");
}

#[tokio::test]
async fn bulk_dml_rejects_unsupported_clauses() {
    let mut ctx = make_ctx();
    ctx.set::<QvRow>();

    let error = ctx
        .set::<QvRow>()
        .query()
        .order_by(QvRow::COLUMN_ID)
        .execute_delete()
        .await
        .expect_err("ORDER BY in bulk DELETE must be rejected");
    assert!(matches!(error, EFError::Configuration(..)), "{error:?}");

    let error = ctx
        .set::<QvRow>()
        .query()
        .join::<QvOther>(JoinKind::Inner, &[("id", "id")])
        .execute_update()
        .set_column(QvRow::COLUMN_LABEL, "x")
        .compile_sql()
        .expect_err("JOIN in bulk UPDATE must be rejected");
    assert!(matches!(error, EFError::Configuration(..)), "{error:?}");

    let error = ctx
        .set::<QvRow>()
        .query()
        .take(10)
        .execute_delete()
        .await
        .expect_err("take in bulk DELETE must be rejected");
    assert!(matches!(error, EFError::Configuration(..)), "{error:?}");
}

#[test]
fn raw_fragment_lexer_ignores_placeholders_inside_literals() {
    // `?` and `$1` inside string literals/dollar-quoted strings must not be
    // treated as placeholders.
    let query = QueryBuilder::<QvRow>::new("qv_rows")
        .with_cte_internal(
            "lit",
            "SELECT *, '?' AS q, $tag$ ? $tag$ AS d FROM qv_rows WHERE amount > ?",
            vec![DbValue::I64(10)],
            &[],
        )
        .from_cte("lit");
    let (sql, params) = query
        .state()
        .compile_sql_with::<QvRow>(&PgLikeGenerator)
        .expect("compile");
    assert!(
        sql.contains("SELECT *, '?' AS q, $tag$ ? $tag$ AS d FROM qv_rows WHERE amount > $1"),
        "literals must stay untouched: {sql}"
    );
    assert_eq!(params, vec![DbValue::I64(10)]);
}

#[test]
fn raw_fragment_double_question_mark_escapes_the_literal_operator() {
    // PostgreSQL's `jsonb ?` operator is written as `??` in a raw fragment.
    let query = QueryBuilder::<QvRow>::new("qv_rows")
        .with_cte_internal(
            "doc",
            "SELECT * FROM qv_rows WHERE data ?? 'key'",
            Vec::new(),
            &[],
        )
        .from_cte("doc");
    let (sql, params) = query
        .state()
        .compile_sql_with::<QvRow>(&PgLikeGenerator)
        .expect("compile");
    assert!(sql.contains("WHERE data ? 'key'"), "unexpected SQL: {sql}");
    assert!(params.is_empty());
}

#[test]
fn unterminated_raw_literal_is_a_typed_error() {
    let query = QueryBuilder::<QvRow>::new("qv_rows").with_cte_internal(
        "bad",
        "SELECT * FROM qv_rows WHERE label = 'oops",
        Vec::new(),
        &[],
    );
    let error = query
        .state()
        .try_compile_with(&PgLikeGenerator)
        .expect_err("unterminated literal must be rejected");
    assert!(matches!(error, EFError::Configuration(..)), "{error:?}");
}
