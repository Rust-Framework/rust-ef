//! Regression tests for v1.1 HAVING placeholder and LIMIT/OFFSET pagination
//! dialect fixes.
//!
//! These tests verify that:
//! - `HavingExpr::to_sql` uses the provider-specific placeholder syntax
//!   (`?` for SQLite/MySQL, `$N` for PostgreSQL) instead of hardcoding `?`.
//! - The PostgreSQL `$N` index continues contiguously from the WHERE clause
//!   into the HAVING clause (e.g. WHERE `$1` → HAVING `$2`).
//! - `QueryState::to_sql_with` delegates LIMIT/OFFSET generation to
//!   `gen.pagination()` so each dialect emits its correct clause order:
//!     - SQLite/MySQL: `LIMIT x OFFSET y`
//!     - PostgreSQL: `OFFSET y LIMIT x`
//!     - MySQL offset-only: `LIMIT 18446744073709551615 OFFSET y`
//!
//! See `迭代计划_v1.1_plus_plan.md` tasks 3.4.1, 3.4.2, 3.4.3.

use rust_ef::provider::{DbValue, ISqlGenerator};
use rust_ef::query::{AggKind, BoolExpr, CompareOp, FilterCondition, HavingExpr, QueryState};

// ---------------------------------------------------------------------------
// Mock generators mimicking each provider's dialect behavior
// ---------------------------------------------------------------------------

/// Mimics PostgreSQL: `$N` placeholders, `OFFSET x LIMIT y` pagination order.
struct PgLikeGenerator;

impl ISqlGenerator for PgLikeGenerator {
    fn select(&self, _: &str, _: &[&str]) -> String {
        String::new()
    }
    fn insert(&self, _: &str, _: &[&str], _: bool) -> String {
        String::new()
    }
    fn update(&self, _: &str, _: &[&str], _: &str) -> String {
        String::new()
    }
    fn delete(&self, _: &str, _: &str) -> String {
        String::new()
    }
    fn create_table(&self, _: &str, _: &[(String, String)]) -> String {
        String::new()
    }
    fn drop_table(&self, _: &str) -> String {
        String::new()
    }
    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String {
        match (skip, take) {
            (Some(s), Some(t)) => format!("OFFSET {} LIMIT {}", s, t),
            (Some(s), None) => format!("OFFSET {}", s),
            (None, Some(t)) => format!("LIMIT {}", t),
            (None, None) => String::new(),
        }
    }
    fn parameter_placeholder(&self, index: usize) -> String {
        format!("${}", index)
    }
    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }
    fn auto_increment_syntax(&self) -> &'static str {
        "SERIAL"
    }
}

/// Mimics MySQL: `?` placeholders, `LIMIT x OFFSET y` order, special
/// offset-only case using `LIMIT 18446744073709551615 OFFSET y`.
struct MySqlLikeGenerator;

impl ISqlGenerator for MySqlLikeGenerator {
    fn select(&self, _: &str, _: &[&str]) -> String {
        String::new()
    }
    fn insert(&self, _: &str, _: &[&str], _: bool) -> String {
        String::new()
    }
    fn update(&self, _: &str, _: &[&str], _: &str) -> String {
        String::new()
    }
    fn delete(&self, _: &str, _: &str) -> String {
        String::new()
    }
    fn create_table(&self, _: &str, _: &[(String, String)]) -> String {
        String::new()
    }
    fn drop_table(&self, _: &str) -> String {
        String::new()
    }
    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String {
        match (skip, take) {
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            (Some(s), None) => format!("LIMIT 18446744073709551615 OFFSET {}", s),
            (None, None) => String::new(),
        }
    }
    fn parameter_placeholder(&self, _: usize) -> String {
        "?".to_string()
    }
    fn quote_identifier(&self, identifier: &str) -> String {
        format!("`{}`", identifier)
    }
    fn auto_increment_syntax(&self) -> &'static str {
        "AUTO_INCREMENT"
    }
}

/// Mimics SQLite: `?` placeholders, `LIMIT x OFFSET y` order, no offset-only.
struct SqliteLikeGenerator;

impl ISqlGenerator for SqliteLikeGenerator {
    fn select(&self, _: &str, _: &[&str]) -> String {
        String::new()
    }
    fn insert(&self, _: &str, _: &[&str], _: bool) -> String {
        String::new()
    }
    fn update(&self, _: &str, _: &[&str], _: &str) -> String {
        String::new()
    }
    fn delete(&self, _: &str, _: &str) -> String {
        String::new()
    }
    fn create_table(&self, _: &str, _: &[(String, String)]) -> String {
        String::new()
    }
    fn drop_table(&self, _: &str) -> String {
        String::new()
    }
    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String {
        match (skip, take) {
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            _ => String::new(),
        }
    }
    fn parameter_placeholder(&self, _: usize) -> String {
        "?".to_string()
    }
    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }
    fn auto_increment_syntax(&self) -> &'static str {
        "AUTOINCREMENT"
    }
}

// ---------------------------------------------------------------------------
// HAVING placeholder dialect tests
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// LIMIT / OFFSET pagination dialect tests (task 3.4.2)
// ---------------------------------------------------------------------------

#[test]
fn test_pg_limit_offset_clause_order() {
    // PostgreSQL emits `OFFSET y LIMIT x` (not `LIMIT x OFFSET y`).
    let mut state = QueryState::new("blogs");
    state.offset = Some(10);
    state.limit = Some(5);

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("OFFSET 10 LIMIT 5"),
        "PG pagination should be `OFFSET 10 LIMIT 5`, got: {sql}"
    );
    assert!(
        !sql.contains("LIMIT 5 OFFSET 10"),
        "PG must NOT use `LIMIT x OFFSET y` order, got: {sql}"
    );
}

#[test]
fn test_pg_offset_only() {
    // PG offset-only: `OFFSET y` with no LIMIT.
    let mut state = QueryState::new("blogs");
    state.offset = Some(20);

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("OFFSET 20") && !sql.contains("LIMIT"),
        "PG offset-only should be `OFFSET 20` with no LIMIT, got: {sql}"
    );
}

#[test]
fn test_pg_limit_only() {
    let mut state = QueryState::new("blogs");
    state.limit = Some(5);

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("LIMIT 5") && !sql.contains("OFFSET"),
        "PG limit-only should be `LIMIT 5`, got: {sql}"
    );
}

#[test]
fn test_sqlite_limit_offset_clause_order() {
    // SQLite emits `LIMIT x OFFSET y`.
    let mut state = QueryState::new("blogs");
    state.offset = Some(10);
    state.limit = Some(5);

    let sql = state.to_sql_with(&SqliteLikeGenerator);
    assert!(
        sql.contains("LIMIT 5 OFFSET 10"),
        "SQLite pagination should be `LIMIT 5 OFFSET 10`, got: {sql}"
    );
}

#[test]
fn test_sqlite_limit_only() {
    let mut state = QueryState::new("blogs");
    state.limit = Some(5);

    let sql = state.to_sql_with(&SqliteLikeGenerator);
    assert!(
        sql.contains("LIMIT 5"),
        "SQLite limit-only should be `LIMIT 5`, got: {sql}"
    );
}

#[test]
fn test_mysql_limit_offset_clause_order() {
    // MySQL emits `LIMIT x OFFSET y`.
    let mut state = QueryState::new("blogs");
    state.offset = Some(10);
    state.limit = Some(5);

    let sql = state.to_sql_with(&MySqlLikeGenerator);
    assert!(
        sql.contains("LIMIT 5 OFFSET 10"),
        "MySQL pagination should be `LIMIT 5 OFFSET 10`, got: {sql}"
    );
}

#[test]
fn test_mysql_offset_only_uses_large_limit() {
    // MySQL offset-only special case: `LIMIT 18446744073709551615 OFFSET y`.
    let mut state = QueryState::new("blogs");
    state.offset = Some(20);

    let sql = state.to_sql_with(&MySqlLikeGenerator);
    assert!(
        sql.contains("LIMIT 18446744073709551615 OFFSET 20"),
        "MySQL offset-only should use large LIMIT, got: {sql}"
    );
}

#[test]
fn test_no_pagination_emits_nothing() {
    let state = QueryState::new("blogs");
    // No limit, no offset → no pagination clause at all.
    let sql_pg = state.to_sql_with(&PgLikeGenerator);
    let sql_sqlite = state.to_sql_with(&SqliteLikeGenerator);
    let sql_mysql = state.to_sql_with(&MySqlLikeGenerator);
    assert!(
        !sql_pg.contains("LIMIT") && !sql_pg.contains("OFFSET"),
        "No pagination should emit nothing (PG), got: {sql_pg}"
    );
    assert!(
        !sql_sqlite.contains("LIMIT") && !sql_sqlite.contains("OFFSET"),
        "No pagination should emit nothing (SQLite), got: {sql_sqlite}"
    );
    assert!(
        !sql_mysql.contains("LIMIT") && !sql_mysql.contains("OFFSET"),
        "No pagination should emit nothing (MySQL), got: {sql_mysql}"
    );
}

// ---------------------------------------------------------------------------
// Combined WHERE + HAVING + LIMIT/OFFSET end-to-end SQL tests
// ---------------------------------------------------------------------------

#[test]
fn test_pg_full_query_where_having_pagination() {
    // Full query: WHERE category = $1 GROUP BY ... HAVING COUNT(id) > $2
    //             OFFSET 10 LIMIT 5
    let mut state = QueryState::new("blogs");
    state.parameters.push(DbValue::String("tech".to_string()));
    state.parameters.push(DbValue::I32(1));
    state.where_expr = Some(BoolExpr::Filter(FilterCondition::new("category", "=", 1)));
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    });
    state.offset = Some(10);
    state.limit = Some(5);

    let sql = state.to_sql_with(&PgLikeGenerator);
    assert!(sql.contains("WHERE category = $1"), "WHERE: {sql}");
    assert!(sql.contains("GROUP BY category"), "GROUP BY: {sql}");
    assert!(
        sql.contains("HAVING COUNT(id) > $2"),
        "HAVING should continue at $2: {sql}"
    );
    assert!(
        sql.contains("OFFSET 10 LIMIT 5"),
        "PG pagination should be OFFSET then LIMIT: {sql}"
    );
}

#[test]
fn test_mysql_full_query_where_having_pagination() {
    // MySQL: WHERE category = ? GROUP BY ... HAVING COUNT(id) > ?
    //        LIMIT 5 OFFSET 10
    let mut state = QueryState::new("blogs");
    state.parameters.push(DbValue::String("tech".to_string()));
    state.parameters.push(DbValue::I32(1));
    state.where_expr = Some(BoolExpr::Filter(FilterCondition::new("category", "=", 1)));
    state.group_bys = vec!["category".to_string()];
    state.havings.push(HavingExpr::Compare {
        agg: AggKind::Count,
        col: "id".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I32(1),
    });
    state.offset = Some(10);
    state.limit = Some(5);

    let sql = state.to_sql_with(&MySqlLikeGenerator);
    assert!(sql.contains("WHERE category = ?"), "WHERE: {sql}");
    assert!(sql.contains("HAVING COUNT(id) > ?"), "HAVING: {sql}");
    assert!(
        sql.contains("LIMIT 5 OFFSET 10"),
        "MySQL pagination should be LIMIT then OFFSET: {sql}"
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
