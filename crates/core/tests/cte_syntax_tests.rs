//! Integration tests for the `linq!` macro CTE syntax sugar (`with` / `from`).
//!
//! Verifies that `linq!(with name as |e: T| ...)` compiles the closure body
//! into a `BoolExpr` and generates a typed CTE whose body
//! `SELECT * FROM <table> WHERE <expr>` uses provider-correct placeholders.
//! Also tests execution against SQLite (CTEs are supported since 3.8.3+).

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::linq;
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("cte_employees")]
struct CteEmployee {
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
    ctx.set::<CteEmployee>();
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
        ctx.set::<CteEmployee>().add(CteEmployee {
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
fn test_typed_cte_sql_generation() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<CteEmployee>();
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        from high_earners
    )
    .to_sql();

    assert!(
        sql.contains("WITH high_earners AS ("),
        "expected `WITH high_earners AS (...)` in SQL, got: {sql}"
    );
    assert!(
        sql.contains("SELECT * FROM \"cte_employees\" WHERE"),
        "expected typed CTE body `SELECT * FROM \"cte_employees\" WHERE ...`, got: {sql}"
    );
    // SQLite uses ? placeholders via PortablePlaceholderGenerator.
    assert!(
        sql.contains("?"),
        "expected ? placeholder in typed CTE body, got: {sql}"
    );
    assert!(
        sql.contains("FROM high_earners"),
        "expected main query to reference CTE by name, got: {sql}"
    );
}

#[test]
fn test_typed_cte_compound_where() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<CteEmployee>();
        with eng_high as |e: CteEmployee| e.salary > 85_000 && e.dept == "Engineering";
        from eng_high
    )
    .to_sql();

    assert!(
        sql.contains("WITH eng_high AS ("),
        "expected `WITH eng_high AS (...)`, got: {sql}"
    );
    // Compound WHERE should produce AND in the CTE body.
    assert!(
        sql.contains("AND"),
        "expected AND in compound WHERE CTE body, got: {sql}"
    );
    assert!(
        sql.contains("salary") && sql.contains("dept"),
        "expected column names in CTE body, got: {sql}"
    );
}

#[test]
fn test_typed_cte_multiple_ctes() {
    let mut ctx = build_ctx();
    let sql = linq!(
        ctx.set::<CteEmployee>();
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        with eng_earners as |e: CteEmployee| e.dept == "Engineering";
        from high_earners
    )
    .to_sql();

    assert!(
        sql.contains("WITH high_earners AS ("),
        "expected first CTE, got: {sql}"
    );
    assert!(
        sql.contains(", eng_earners AS ("),
        "expected second CTE with comma separator, got: {sql}"
    );
    assert!(
        sql.contains("\"cte_employees\""),
        "expected both CTEs to reference the source table, got: {sql}"
    );
}

#[test]
fn test_typed_cte_parameter_ordering() {
    let mut ctx = build_ctx();
    // Two typed CTEs each with one parameter, plus a WHERE on the main query.
    // The all_params() should return: [cte1_param, cte2_param, main_param].
    let query = linq!(
        ctx.set::<CteEmployee>(),
        |e: CteEmployee| e.emp_id > 0;
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        from high_earners
    );

    let params = query.state().all_params();
    assert_eq!(
        params.len(),
        2,
        "expected 2 params (CTE + main WHERE), got {}",
        params.len()
    );
    // CTE param comes first. `85_000` literal infers as i32.
    assert!(
        matches!(params[0], DbValue::I32(85_000)),
        "expected CTE param (85000) first, got {:?}",
        params[0]
    );
    // Main query param second. `0` literal infers as i32.
    assert!(
        matches!(params[1], DbValue::I32(0)),
        "expected main query param (0) second, got {:?}",
        params[1]
    );
}

// ---------------------------------------------------------------------------
// Execution tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_typed_cte_execution() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    let employees = linq!(
        ctx.set::<CteEmployee>();
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        from high_earners
    )
    .to_list()
    .await
    .unwrap();

    // salary > 85000: Alice(100k), Bob(90k), Carol(110k), Eve(85k excluded — strictly >).
    assert_eq!(employees.len(), 3, "typed CTE should return 3 high earners");
}

#[tokio::test]
async fn test_typed_cte_compound_execution() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    let employees = linq!(
        ctx.set::<CteEmployee>();
        with eng_high as |e: CteEmployee| e.salary > 85_000 && e.dept == "Engineering";
        from eng_high
    )
    .to_list()
    .await
    .unwrap();

    // Engineering + salary > 85000: Alice(100k), Bob(90k), Carol(110k) → 3 rows.
    assert_eq!(
        employees.len(),
        3,
        "compound CTE should return 3 Engineering high earners"
    );
}

#[tokio::test]
async fn test_typed_cte_with_main_where() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // CTE filters to high earners, main query further filters by dept.
    let employees = linq!(
        ctx.set::<CteEmployee>(),
        |e: CteEmployee| e.dept == "Engineering";
        with high_earners as |e: CteEmployee| e.salary > 80_000;
        from high_earners
    )
    .to_list()
    .await
    .unwrap();

    // CTE: salary > 80000 → Alice, Bob, Carol, Eve (4 rows, excludes Dave & Frank at 80k).
    // Main WHERE: dept == "Engineering" → Alice, Bob, Carol (3 rows).
    assert_eq!(
        employees.len(),
        3,
        "CTE + main WHERE should return 3 Engineering high earners"
    );
}

#[tokio::test]
async fn test_typed_cte_with_order_by() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    let employees = linq!(
        ctx.set::<CteEmployee>();
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        from high_earners;
        order_by e.salary desc
    )
    .to_list()
    .await
    .unwrap();

    assert_eq!(employees.len(), 3);
    // Ordered by salary desc: Carol(110k), Alice(100k), Bob(90k).
    assert_eq!(employees[0].name, "Carol");
    assert_eq!(employees[1].name, "Alice");
    assert_eq!(employees[2].name, "Bob");
}

#[tokio::test]
async fn test_typed_cte_with_or_condition() {
    let mut ctx = build_ctx();
    seed(&mut ctx).await;

    // CTE with OR: salary > 100000 OR dept == "Sales".
    let employees = linq!(
        ctx.set::<CteEmployee>();
        with special as |e: CteEmployee| e.salary > 100_000 || e.dept == "Sales";
        from special
    )
    .to_list()
    .await
    .unwrap();

    // salary > 100000: Carol(110k) → 1
    // dept == "Sales": Dave, Eve, Frank → 3
    // Total: 4 (no overlap).
    assert_eq!(
        employees.len(),
        4,
        "OR condition CTE should return 4 employees"
    );
}

// ---------------------------------------------------------------------------
// PostgreSQL dialect tests
// ---------------------------------------------------------------------------
//
// Regression coverage for a v1.1 bug where multiple typed CTEs reset
// `cte_idx` to 1 inside the `.map()` closure, producing duplicate `$1`
// placeholders across CTEs on PostgreSQL. The fix uses a `running_idx` that
// accumulates across CTEs so `$N` stays contiguous with `all_params()` order.
//
// These tests use a `PgLikeGenerator` mock so they run without a live
// PostgreSQL instance.

use rust_ef::provider::ISqlGenerator;

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
    fn pagination(&self, _: Option<usize>, _: Option<usize>) -> String {
        String::new()
    }
    fn parameter_placeholder(&self, index: usize) -> String {
        format!("${index}")
    }
    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{identifier}\"")
    }
    fn auto_increment_syntax(&self) -> &'static str {
        "SERIAL"
    }
}

#[test]
fn test_pg_single_typed_cte_uses_dollar_n() {
    // Single typed CTE on PostgreSQL: body should use `$1`, not `?`.
    let mut ctx = build_ctx();
    let query = linq!(
        ctx.set::<CteEmployee>();
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        from high_earners
    );

    let sql = query.state().to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("WHERE salary > $1"),
        "PG typed CTE body should use $1 placeholder, got: {sql}"
    );
    assert!(
        !sql.contains('?'),
        "PG typed CTE body must not contain `?` placeholder, got: {sql}"
    );
}

#[test]
fn test_pg_multiple_typed_ctes_contiguous_placeholders() {
    // Regression: two typed CTEs each with one parameter. Before the fix,
    // both emitted `$1` because `cte_idx` reset per CTE. After the fix,
    // placeholders should be `$1` (first CTE) and `$2` (second CTE).
    let mut ctx = build_ctx();
    let query = linq!(
        ctx.set::<CteEmployee>();
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        with eng_earners as |e: CteEmployee| e.dept == "Engineering";
        from high_earners
    );

    let sql = query.state().to_sql_with(&PgLikeGenerator);

    // First CTE body: salary > $1
    assert!(
        sql.contains("salary > $1"),
        "first CTE should use $1, got: {sql}"
    );
    // Second CTE body: dept = $2 (NOT $1 — that was the bug)
    assert!(
        sql.contains("dept = $2"),
        "second CTE should use $2 (regression: was $1 before fix), got: {sql}"
    );
    // The string `$1, $1` or `$1) AND ... $1` would indicate the collision.
    // Verify `$2` appears and the second CTE doesn't reuse `$1`.
    let second_cte_start = sql
        .find("eng_earners AS (")
        .unwrap_or_else(|| panic!("expected `eng_earners AS (` in SQL, got: {sql}"));
    let second_cte_body = &sql[second_cte_start..];
    assert!(
        !second_cte_body.contains("= $1"),
        "second CTE must not reuse $1 (regression), got: {sql}"
    );
    assert!(
        second_cte_body.contains("= $2"),
        "second CTE should use $2, got: {sql}"
    );
}

#[test]
fn test_pg_multi_cte_with_main_where_contiguous() {
    // Three param slots across the query: 2 CTE params + 1 main WHERE param.
    // Order in `all_params()`: [cte1_param, cte2_param, main_param].
    // Placeholders must be: CTE1 → $1, CTE2 → $2, main WHERE → $3.
    let mut ctx = build_ctx();
    let query = linq!(
        ctx.set::<CteEmployee>(),
        |e: CteEmployee| e.emp_id > 0;
        with high_earners as |e: CteEmployee| e.salary > 85_000;
        with eng_earners as |e: CteEmployee| e.dept == "Engineering";
        from high_earners
    );

    let sql = query.state().to_sql_with(&PgLikeGenerator);

    // Verify placeholder continuity: $1 (CTE1), $2 (CTE2), $3 (main WHERE).
    assert!(
        sql.contains("salary > $1"),
        "CTE1 should use $1, got: {sql}"
    );
    assert!(sql.contains("dept = $2"), "CTE2 should use $2, got: {sql}");
    assert!(
        sql.contains("emp_id > $3"),
        "main WHERE should use $3 (continuity from CTEs), got: {sql}"
    );

    // Verify all_params() ordering matches the physical SQL order.
    let params = query.state().all_params();
    assert_eq!(
        params.len(),
        3,
        "expected 3 params (2 CTE + 1 main), got {}",
        params.len()
    );
    assert!(
        matches!(params[0], DbValue::I32(85_000)),
        "param[0] should be CTE1's 85000, got {:?}",
        params[0]
    );
    assert!(
        matches!(params[1], DbValue::String(_)),
        "param[1] should be CTE2's dept string, got {:?}",
        params[1]
    );
    assert!(
        matches!(params[2], DbValue::I32(0)),
        "param[2] should be main WHERE's 0, got {:?}",
        params[2]
    );
}

#[test]
fn test_pg_compound_where_cte_placeholder_count() {
    // A single typed CTE with a compound WHERE (2 params) on PostgreSQL.
    // Body should be `WHERE (salary > $1) AND (dept = $2)`.
    let mut ctx = build_ctx();
    let query = linq!(
        ctx.set::<CteEmployee>();
        with eng_high as |e: CteEmployee| e.salary > 85_000 && e.dept == "Engineering";
        from eng_high
    );

    let sql = query.state().to_sql_with(&PgLikeGenerator);
    assert!(
        sql.contains("salary > $1"),
        "compound CTE first param should be $1, got: {sql}"
    );
    assert!(
        sql.contains("dept = $2"),
        "compound CTE second param should be $2, got: {sql}"
    );
    assert!(
        !sql.contains("$3"),
        "compound CTE with 2 params must not emit $3, got: {sql}"
    );
}
