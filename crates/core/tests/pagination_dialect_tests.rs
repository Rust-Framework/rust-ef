//! LIMIT/OFFSET pagination dialect and combined end-to-end regression tests.
//!
//! Verifies that `QueryState::to_sql_with` delegates LIMIT/OFFSET generation
//! to `gen.pagination()` so each dialect emits its correct clause order:
//!   - SQLite/MySQL: `LIMIT x OFFSET y`
//!   - PostgreSQL: `OFFSET y LIMIT x`
//!   - MySQL offset-only: `LIMIT 18446744073709551615 OFFSET y`

mod common;

use common::{MySqlLikeGenerator, PgLikeGenerator, SqliteLikeGenerator};
use rust_ef::provider::DbValue;
use rust_ef::query::{AggKind, BoolExpr, CompareOp, FilterCondition, HavingExpr, QueryState};

// ---------------------------------------------------------------------------
// LIMIT / OFFSET pagination dialect tests
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
