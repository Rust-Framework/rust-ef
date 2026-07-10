//! Integration tests for v1.1 window function and CTE support.
//!
//! Verifies that `linq!(window ...)` generates correct `FUNC() OVER (...)`
//! SQL and that `with_cte_internal` emits a `WITH name AS (...)` prefix.
//! Execution tests confirm the queries run against SQLite (window functions
//! are supported since SQLite 3.25+, CTEs since 3.8.3+).

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("win_employees")]
struct WinEmployee {
    #[primary_key]
    #[auto_increment]
    emp_id: i32,
    #[required]
    name: String,
    #[required]
    dept: String,
    #[required]
    salary: i64,
}

fn build_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("DbContext")
}

async fn seed(ctx: &mut DbContext) {
    ctx.set::<WinEmployee>();
    ctx.ensure_created().await.unwrap();

    let employees = [
        ("Alice", "Engineering", 100_000),
        ("Bob", "Engineering", 90_000),
        ("Carol", "Engineering", 110_000),
        ("Dave", "Sales", 80_000),
        ("Eve", "Sales", 85_000),
        ("Frank", "Sales", 80_000),
    ];

    for (name, dept, salary) in &employees {
        ctx.add::<WinEmployee>(WinEmployee {
            emp_id: 0,
            name: (*name).into(),
            dept: (*dept).into(),
            salary: *salary,
        });
    }
    ctx.save_changes().await.unwrap();
}

// ---------------------------------------------------------------------------
// SQL generation tests
// ---------------------------------------------------------------------------

#[test]
fn test_row_number_sql_generation() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<WinEmployee>(),
        |e: WinEmployee| e.emp_id > 0;
        window row_number partition_by e.dept order_by e.salary desc as rn
    )
    .to_sql();

    assert!(
        sql.contains("ROW_NUMBER()"),
        "expected ROW_NUMBER() in SQL, got: {sql}"
    );
    assert!(
        sql.contains("PARTITION BY"),
        "expected PARTITION BY in SQL, got: {sql}"
    );
    assert!(
        sql.contains("ORDER BY") && sql.contains("\"salary\" DESC"),
        "expected ORDER BY \"salary\" DESC in SQL, got: {sql}"
    );
    assert!(
        sql.contains("AS \"rn\""),
        "expected AS \"rn\" alias in SQL, got: {sql}"
    );
}

#[test]
fn test_sum_window_sql_generation() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<WinEmployee>();
        window sum e.salary partition_by e.dept as dept_total
    )
    .to_sql();

    assert!(
        sql.contains("SUM(\"salary\")"),
        "expected SUM(\"salary\") in SQL, got: {sql}"
    );
    assert!(
        sql.contains("PARTITION BY \"dept\""),
        "expected PARTITION BY \"dept\" in SQL, got: {sql}"
    );
    assert!(
        sql.contains("AS \"dept_total\""),
        "expected AS \"dept_total\" in SQL, got: {sql}"
    );
}

#[test]
fn test_rank_window_no_partition() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<WinEmployee>();
        window rank order_by e.salary desc as salary_rank
    )
    .to_sql();

    assert!(sql.contains("RANK()"), "expected RANK() in SQL, got: {sql}");
    assert!(
        !sql.contains("PARTITION BY"),
        "RANK without partition_by should not emit PARTITION BY, got: {sql}"
    );
    assert!(
        sql.contains("ORDER BY \"salary\" DESC"),
        "expected ORDER BY \"salary\" DESC in SQL, got: {sql}"
    );
}

#[test]
fn test_lag_window_sql_generation() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<WinEmployee>();
        window lag e.salary order_by e.emp_id asc as prev_salary
    )
    .to_sql();

    assert!(
        sql.contains("LAG(\"salary\")"),
        "expected LAG(\"salary\") in SQL, got: {sql}"
    );
    assert!(
        sql.contains("ORDER BY \"emp_id\" ASC"),
        "expected ORDER BY \"emp_id\" ASC in SQL, got: {sql}"
    );
}

#[test]
fn test_multiple_windows_in_one_query() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<WinEmployee>();
        window row_number partition_by e.dept order_by e.salary desc as rn;
        window sum e.salary partition_by e.dept as dept_total
    )
    .to_sql();

    assert!(
        sql.contains("ROW_NUMBER()"),
        "expected ROW_NUMBER() in SQL, got: {sql}"
    );
    assert!(
        sql.contains("SUM(\"salary\")"),
        "expected SUM(\"salary\") in SQL, got: {sql}"
    );
    assert!(
        sql.contains("AS \"rn\""),
        "expected AS \"rn\" in SQL, got: {sql}"
    );
    assert!(
        sql.contains("AS \"dept_total\""),
        "expected AS \"dept_total\" in SQL, got: {sql}"
    );
}

#[test]
fn test_dense_rank_window_sql_generation() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<WinEmployee>();
        window dense_rank partition_by e.dept order_by e.salary desc as dr
    )
    .to_sql();

    assert!(
        sql.contains("DENSE_RANK()"),
        "expected DENSE_RANK() in SQL, got: {sql}"
    );
}

// ---------------------------------------------------------------------------
// Execution tests (SQLite supports window functions since 3.25+)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_row_number_execution() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC)
    // Should assign 1,2,3 to Engineering (by salary desc) and 1,2,3 to Sales.
    let employees = linq!(
        ctx.set::<WinEmployee>(),
        |e: WinEmployee| e.emp_id > 0;
        window row_number partition_by e.dept order_by e.salary desc as rn
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(
        employees.len(),
        6,
        "expected 6 employees with window projection"
    );
}

#[tokio::test]
async fn test_sum_window_execution() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // SUM(salary) OVER (PARTITION BY dept)
    // Engineering total = 300000, Sales total = 245000.
    let employees = linq!(
        ctx.set::<WinEmployee>();
        window sum e.salary partition_by e.dept as dept_total
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(employees.len(), 6);
    // from_row ignores the extra window column, so entity fields are intact.
    let eng: Vec<&WinEmployee> = employees
        .iter()
        .filter(|e| e.dept == "Engineering")
        .collect();
    assert_eq!(eng.len(), 3);
}

#[tokio::test]
async fn test_window_with_where_filter() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // Window function combined with a WHERE filter.
    let employees = linq!(
        ctx.set::<WinEmployee>(),
        |e: WinEmployee| e.salary > 80_000;
        window rank order_by e.salary desc as r
    )
    .to_list()
    .await
    .unwrap();

    // salary > 80000: Alice(100k), Bob(90k), Carol(110k), Eve(85k) → 4 rows.
    assert_eq!(employees.len(), 4);
}

// ---------------------------------------------------------------------------
// CTE tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_cte_with_parameters() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // CTE: high_earners AS (SELECT * FROM win_employees WHERE salary > ?)
    // Main query: SELECT * FROM high_earners
    let employees = ctx
        .set::<WinEmployee>()
        .query()
        .with_cte_internal(
            "high_earners",
            "SELECT * FROM win_employees WHERE salary > ?",
            vec![DbValue::I64(85_000)],
            &[],
        )
        .from_cte("high_earners")
        .to_list()
        .await
        .unwrap();

    // salary > 85000: Alice(100k), Bob(90k), Carol(110k) → 3 rows.
    assert_eq!(
        employees.len(),
        3,
        "CTE with parameter should return 3 high earners"
    );
}

#[tokio::test]
async fn test_cte_sql_generation() {
    let mut ctx = build_ctx();

    let sql = ctx
        .set::<WinEmployee>()
        .query()
        .with_cte_internal(
            "high_earners",
            "SELECT * FROM win_employees WHERE salary > ?",
            vec![DbValue::I64(85_000)],
            &[],
        )
        .from_cte("high_earners")
        .to_sql();

    assert!(
        sql.starts_with("WITH high_earners AS ("),
        "expected WITH prefix in SQL, got: {sql}"
    );
    assert!(
        sql.contains("SELECT * FROM win_employees WHERE salary > ?"),
        "expected CTE body in SQL, got: {sql}"
    );
    assert!(
        sql.contains("FROM high_earners"),
        "expected FROM high_earners in main query, got: {sql}"
    );
}

#[tokio::test]
async fn test_cte_with_explicit_columns() {
    let mut ctx = build_ctx();

    let sql = ctx
        .set::<WinEmployee>()
        .query()
        .with_cte_internal(
            "ranked",
            "SELECT emp_id, name FROM win_employees",
            vec![],
            &["emp_id", "name"],
        )
        .from_cte("ranked")
        .to_sql();

    assert!(
        sql.contains("ranked (\"emp_id\", \"name\") AS"),
        "expected CTE with explicit columns in SQL, got: {sql}"
    );
}
