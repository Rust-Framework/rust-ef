//! HAVING placeholder dialect regression tests.
//!
//! Verifies that `HavingExpr::to_sql` uses the provider-specific placeholder
//! syntax (`?` for SQLite/MySQL, `$N` for PostgreSQL) instead of hardcoding `?`.
//! Also verifies the PostgreSQL `$N` index continues contiguously from the
//! WHERE clause into the HAVING clause (e.g. WHERE `$1` → HAVING `$2`).

mod common;

use common::{MySqlLikeGenerator, PgLikeGenerator, SqliteLikeGenerator};
use rust_ef::provider::DbValue;
use rust_ef::query::{AggKind, BoolExpr, CompareOp, FilterCondition, HavingExpr, QueryState};

#[test]
fn test_pg_having_uses_dollar_n_placeholder() {
    // HAVING COUNT(blog_id) > 1 — should emit `$1` on PostgreSQL, not `?`.
    let mut state = QueryState::new("blogs");
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "blog_id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    });
    state.parameters.push(DbValue::I32(1));

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("HAVING COUNT(blog_id) > $1"),
        "PG HAVING should use $1 placeholder, got: {sql}"
    );
    assert!(
        !sql.contains("?"),
        "PG HAVING must not contain `?` placeholder, got: {sql}"
    );
}

#[test]
fn test_sqlite_having_uses_question_mark_placeholder() {
    let mut state = QueryState::new("blogs");
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "blog_id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    });
    state.parameters.push(DbValue::I32(1));

    let sql = state.to_sql_with(&SqliteLikeGenerator);
    assert!(
        sql.contains("HAVING COUNT(blog_id) > ?"),
        "SQLite HAVING should use ? placeholder, got: {sql}"
    );
}

#[test]
fn test_mysql_having_uses_question_mark_placeholder() {
    let mut state = QueryState::new("blogs");
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "blog_id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    });
    state.parameters.push(DbValue::I32(1));

    let sql = state.to_sql_with(&MySqlLikeGenerator);
    assert!(
        sql.contains("HAVING COUNT(blog_id) > ?"),
        "MySQL HAVING should use ? placeholder, got: {sql}"
    );
}

// ---------------------------------------------------------------------------
// PG $N index continuation tests (the core of the bug)
// ---------------------------------------------------------------------------

#[test]
fn test_pg_having_index_continues_from_where() {
    // WHERE category = $1 GROUP BY ... HAVING COUNT(blog_id) > $2
    // The HAVING placeholder must be $2 (continuing from WHERE's $1), not $1.
    let mut state = QueryState::new("blogs");
    state.parameters.push(DbValue::String("tech".to_string()));
    state.parameters.push(DbValue::I32(1));
    state.where_expr = Some(BoolExpr::Filter(FilterCondition::new("category", "=", 1)));
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "blog_id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    });

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("WHERE category = $1"),
        "WHERE should use $1, got: {sql}"
    );
    assert!(
        sql.contains("HAVING COUNT(blog_id) > $2"),
        "HAVING should continue at $2 after WHERE $1, got: {sql}"
    );
}

#[test]
fn test_pg_having_index_continues_through_and_combination() {
    // HAVING COUNT(blog_id) > $1 AND SUM(views) > $2
    // (No WHERE clause, so HAVING starts at $1.)
    let mut state = QueryState::new("blogs");
    state.group_bys = vec!["category".to_string()];
    let expr = HavingExpr::And(
        Box::new(HavingExpr::Compare {
            agg: AggKind::Count,
            col: "blog_id".to_string(),
            op: CompareOp::Gt,
            value: DbValue::I32(1),
        }),
        Box::new(HavingExpr::Compare {
            agg: AggKind::Sum,
            col: "views".to_string(),
            op: CompareOp::Gt,
            value: DbValue::I32(100),
        }),
    );
    state.parameters.extend(expr.collect_params());
    state.havings.push(expr);

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("HAVING (COUNT(blog_id) > $1 AND SUM(views) > $2)"),
        "HAVING AND should use $1 and $2, got: {sql}"
    );
}

#[test]
fn test_pg_having_index_continues_through_or_with_where() {
    // WHERE category = $1 HAVING (COUNT(blog_id) > $2 OR SUM(views) > $3)
    let mut state = QueryState::new("blogs");
    state.parameters.push(DbValue::String("tech".to_string()));
    state.where_expr = Some(BoolExpr::Filter(FilterCondition::new("category", "=", 1)));
    state.group_bys = vec!["category".to_string()];
    let having = HavingExpr::Or(
        Box::new(HavingExpr::Compare {
            agg: AggKind::Count,
            col: "blog_id".to_string(),
            op: CompareOp::Gt,
            value: DbValue::I32(5),
        }),
        Box::new(HavingExpr::Compare {
            agg: AggKind::Sum,
            col: "views".to_string(),
            op: CompareOp::Gt,
            value: DbValue::I32(100),
        }),
    );
    state.parameters.extend(having.collect_params());
    state.havings.push(having);

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("WHERE category = $1"),
        "WHERE should use $1, got: {sql}"
    );
    assert!(
        sql.contains("HAVING (COUNT(blog_id) > $2 OR SUM(views) > $3)"),
        "HAVING OR should continue at $2 and $3, got: {sql}"
    );
}

#[test]
fn test_pg_having_not_preserves_index() {
    // HAVING NOT (COUNT(blog_id) > $1)
    let mut state = QueryState::new("blogs");
    state.group_bys = vec!["category".to_string()];
    let having = HavingExpr::Not(Box::new(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "blog_id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    }));
    state.parameters.extend(having.collect_params());
    state.havings.push(having);

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("HAVING NOT (COUNT(blog_id) > $1)"),
        "HAVING NOT should use $1, got: {sql}"
    );
}

#[test]
fn test_pg_having_compare_agg_emits_no_placeholder() {
    // HAVING SUM(views) > COUNT(blog_id) — no bound params, no placeholders.
    let mut state = QueryState::new("blogs");
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::CompareAgg {
        left_agg: AggKind::Sum,
        left_col: "views".to_string(),
        op: CompareOp::Gt,
        right_agg: AggKind::Count,
        right_col: "blog_id".to_string(),
    });

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("HAVING SUM(views) > COUNT(blog_id)"),
        "HAVING CompareAgg should emit no placeholder, got: {sql}"
    );
    assert!(
        !sql.contains('$'),
        "CompareAgg must not emit any $N placeholder, got: {sql}"
    );
}

#[test]
fn test_pg_having_multiple_clauses_join_with_and() {
    // Two separate having() calls → joined by AND, indices continue.
    let mut state = QueryState::new("blogs");
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "blog_id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    });
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Sum,
        col: "views".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(100),
    });
    state.parameters.push(DbValue::I32(1));
    state.parameters.push(DbValue::I32(100));

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("HAVING COUNT(blog_id) > $1 AND SUM(views) > $2"),
        "Multiple HAVING clauses should join with AND and continue indices, got: {sql}"
    );
}

#[test]
fn test_collect_params_matches_placeholder_order() {
    // Verify that collect_params() returns values in the same left-to-right
    // order that to_sql() emits placeholders.
    let expr = HavingExpr::And(
        Box::new(HavingExpr::Compare {
            agg: AggKind::Count,
            col: "a".to_string(),
            op: CompareOp::Gt,
            value: DbValue::I32(1),
        }),
        Box::new(HavingExpr::Or(
            Box::new(HavingExpr::Compare {
                agg: AggKind::Sum,
                col: "b".to_string(),
                op: CompareOp::Gt,
                value: DbValue::I32(2),
            }),
            Box::new(HavingExpr::Not(Box::new(HavingExpr::Compare {
                agg: AggKind::Avg,
                col: "c".to_string(),
                op: CompareOp::Lt,
                value: DbValue::I32(3),
            }))),
        )),
    );

    let params = expr.collect_params();
    assert_eq!(
        params,
        vec![DbValue::I32(1), DbValue::I32(2), DbValue::I32(3),],
        "collect_params must return values in left-to-right traversal order"
    );

    // The PG SQL should reference $1, $2, $3 in the same order.
    let mut idx = 1usize;
    let sql = expr.to_sql(&PgLikeGenerator, &mut idx);
    assert!(
        sql.contains("$1") && sql.contains("$2") && sql.contains("$3"),
        "to_sql should emit $1, $2, $3 in order, got: {sql}"
    );
    assert_eq!(idx, 4, "param_idx should advance past all 3 params");
}
