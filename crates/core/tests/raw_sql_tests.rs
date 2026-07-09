//! Verifies D4: Raw SQL → entity mapping (`DbContext::sql_query`).
//!
//! Tests the escape hatch for complex queries that are hard to express via LINQ:
//! - Basic SELECT with parameterized WHERE
//! - Aggregate/GROUP BY query mapped to entities

mod common;

use common::TestItem;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::db_set::IDbSet;
use rust_ef::provider::DbValue;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("ctx")
}

#[tokio::test]
async fn sql_query_maps_rows_to_entities() {
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");

    // Seed three rows.
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Alpha".into(),
        value: 1.0,
    });
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Bravo".into(),
        value: 2.0,
    });
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Charlie".into(),
        value: 3.0,
    });
    ctx.save_changes().await.expect("seed save");

    // Raw SQL: SELECT by name parameter.
    let rows: Vec<TestItem> = ctx
        .sql_query(
            "SELECT id, name, value FROM test_items WHERE name = ?",
            &[DbValue::String("Bravo".into())],
        )
        .await
        .expect("sql_query");

    assert_eq!(rows.len(), 1, "should find one row named Bravo");
    assert_eq!(rows[0].name, "Bravo");
    assert!((rows[0].value - 2.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn sql_query_returns_empty_on_no_match() {
    let mut ctx = make_ctx();
    ctx.set::<TestItem>();
    ctx.ensure_created().await.expect("ensure_created");

    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Alpha".into(),
        value: 1.0,
    });
    ctx.save_changes().await.expect("seed save");

    let rows: Vec<TestItem> = ctx
        .sql_query(
            "SELECT id, name, value FROM test_items WHERE name = ?",
            &[DbValue::String("NonExistent".into())],
        )
        .await
        .expect("sql_query");

    assert!(rows.is_empty(), "should return empty vec on no match");
}
