//! EF-05: arbitrary row materialization — `IRow`, `#[derive(FromRow)]`, tuples,
//! `DbContext::sql_query_rows` and `SelectQueryBuilder::to_list_rows`.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::SelectItem;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

/// Report row: not an entity, fields map positionally onto the SELECT list.
#[derive(Debug, Clone, PartialEq, FromRow)]
struct CategoryTotal {
    category: String,
    total: i64,
}

/// Same-typed columns prove that mapping is positional (and that field order
/// matters).
#[derive(Debug, Clone, PartialEq, FromRow)]
struct Pair {
    left: i32,
    right: i32,
}

#[derive(Debug, Clone, EntityType)]
#[table("rowmap_items")]
struct RowItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    category: String,
    amount: i64,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).expect("context")
}

async fn seeded_ctx() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<RowItem>();
    ctx.ensure_created().await.expect("ensure_created");
    for (category, amount) in [("books", 100), ("books", 50), ("toys", 30), ("toys", 20)] {
        ctx.add::<RowItem>(RowItem {
            id: 0,
            category: category.to_string(),
            amount,
        });
    }
    ctx.save_changes().await.expect("seed save");
    ctx.set::<RowItem>().clear_entries();
    ctx
}

// ---------------------------------------------------------------------------
// IRow / derive
// ---------------------------------------------------------------------------

#[test]
fn derived_from_row_maps_values_in_declaration_order() {
    let rows = vec![vec![
        DbValue::String("books".to_string()),
        DbValue::I64(150),
    ]];
    let mapped: Vec<CategoryTotal> = materialize_rows(&rows).expect("materialize");
    assert_eq!(
        mapped,
        vec![CategoryTotal {
            category: "books".to_string(),
            total: 150,
        }]
    );
}

#[test]
fn field_order_is_positional() {
    let direct = Pair::from_row(&[DbValue::I32(10), DbValue::I32(20)]).expect("pair");
    assert_eq!(
        direct,
        Pair {
            left: 10,
            right: 20
        }
    );

    let swapped = Pair::from_row(&[DbValue::I32(20), DbValue::I32(10)]).expect("pair");
    assert_eq!(
        swapped,
        Pair {
            left: 20,
            right: 10
        },
        "positional mapping: the SELECT order is the field order"
    );
}

#[test]
fn from_values_remains_a_compatibility_alias_for_from_row() {
    // 2.8 merged the row-materialization traits; `from_values` survives as a
    // delegating alias of the canonical `from_row`.
    let alias = <Pair as rust_ef::row::IRow>::from_values(&[DbValue::I32(1), DbValue::I32(2)])
        .expect("compatibility alias");
    let canonical = Pair::from_row(&[DbValue::I32(1), DbValue::I32(2)]).expect("canonical");
    assert_eq!(alias, canonical);
}

#[test]
fn short_row_reports_the_missing_field_name_and_index() {
    let err = CategoryTotal::from_row(&[DbValue::String("books".to_string())])
        .expect_err("short row must fail");
    match err {
        EFError::TypeConversion(message, _) => {
            assert!(
                message.contains("missing field `total`"),
                "unexpected message: {message}"
            );
            assert!(
                message.contains("column 1"),
                "unexpected message: {message}"
            );
            assert!(
                message.contains("CategoryTotal"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected TypeConversion, got {other:?}"),
    }
}

#[test]
fn unconvertible_cell_reports_the_field_name_and_index() {
    let err = Pair::from_row(&[DbValue::I32(1), DbValue::String("nope".to_string())])
        .expect_err("bad cell must fail");
    match err {
        EFError::TypeConversion(message, _) => {
            assert!(
                message.contains("field `right`"),
                "unexpected message: {message}"
            );
            assert!(
                message.contains("column 1"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected TypeConversion, got {other:?}"),
    }
}

#[test]
fn tuples_implement_row_for_fixed_arities() {
    let two = <(String, i64)>::from_row(&[DbValue::String("toys".to_string()), DbValue::I64(7)])
        .expect("two-tuple");
    assert_eq!(two, ("toys".to_string(), 7));

    let one = <(i64,)>::from_row(&[DbValue::I64(42)]).expect("one-tuple");
    assert_eq!(one.0, 42);

    type Twelve = (i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32, i32);
    let values: Vec<DbValue> = (1..=12).map(DbValue::I32).collect();
    let twelve = <Twelve>::from_row(&values).expect("twelve-tuple");
    assert_eq!(twelve, (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12));
}

#[test]
fn tuple_arity_mismatch_is_a_typed_error() {
    let short = <(i32, i32)>::from_row(&[DbValue::I32(1)]).expect_err("short tuple");
    match short {
        EFError::TypeConversion(message, _) => {
            assert!(message.contains("needs exactly 2"), "unexpected: {message}");
        }
        other => panic!("expected TypeConversion, got {other:?}"),
    }

    let long = <(i32,)>::from_row(&[DbValue::I32(1), DbValue::I32(2)]).expect_err("long tuple");
    assert!(matches!(long, EFError::TypeConversion(..)));
}

// ---------------------------------------------------------------------------
// sql_query_rows / to_list_rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sql_query_rows_materializes_a_grouped_report() {
    let ctx = seeded_ctx().await;

    let rows: Vec<CategoryTotal> = ctx
        .sql_query_rows(
            "SELECT category, SUM(amount) FROM rowmap_items GROUP BY category ORDER BY category",
            &[],
        )
        .await
        .expect("grouped raw rows");

    assert_eq!(
        rows,
        vec![
            CategoryTotal {
                category: "books".to_string(),
                total: 150,
            },
            CategoryTotal {
                category: "toys".to_string(),
                total: 50,
            },
        ]
    );
}

#[tokio::test]
async fn sql_query_rows_supports_parameters_and_tuple_targets() {
    let ctx = seeded_ctx().await;

    let rows: Vec<(String, i64)> = ctx
        .sql_query_rows(
            "SELECT category, SUM(amount) FROM rowmap_items \
             WHERE amount > ? GROUP BY category ORDER BY category",
            &[DbValue::I64(25)],
        )
        .await
        .expect("parameterized raw rows");

    assert_eq!(
        rows,
        vec![("books".to_string(), 150), ("toys".to_string(), 30),]
    );
}

#[tokio::test]
async fn entities_are_valid_row_targets_through_the_entity_derive() {
    let ctx = seeded_ctx().await;

    let entities: Vec<RowItem> = ctx
        .sql_query_rows("SELECT * FROM rowmap_items ORDER BY id", &[])
        .await
        .expect("entity rows");

    assert_eq!(entities.len(), 4);
    assert_eq!(entities[0].category, "books");
    assert_eq!(entities[0].amount, 100);
}

#[tokio::test]
async fn to_list_rows_materializes_typed_projections() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<CategoryTotal> = ctx
        .set::<RowItem>()
        .query()
        .group_by(&["category"])
        .select_items(&[SelectItem::column("category"), SelectItem::sum("amount")])
        .order_by("category")
        .to_list_rows::<CategoryTotal>()
        .await
        .expect("typed projection rows");

    assert_eq!(
        rows,
        vec![
            CategoryTotal {
                category: "books".to_string(),
                total: 150,
            },
            CategoryTotal {
                category: "toys".to_string(),
                total: 50,
            },
        ]
    );
}

#[tokio::test]
async fn to_list_rows_supports_tuples() {
    let mut ctx = seeded_ctx().await;

    let rows: Vec<(String, i64)> = ctx
        .set::<RowItem>()
        .query()
        .group_by(&["category"])
        .select_items(&[SelectItem::column("category"), SelectItem::count()])
        .order_by("category")
        .to_list_rows::<(String, i64)>()
        .await
        .expect("tuple projection rows");

    assert_eq!(
        rows,
        vec![("books".to_string(), 2), ("toys".to_string(), 2)]
    );
}

#[test]
fn derived_rows_reject_extra_columns_instead_of_ignoring_them() {
    #[derive(Debug, rust_ef::FromRow)]
    struct TwoColumnRow {
        first: i32,
        second: String,
    }

    let too_long = <TwoColumnRow as rust_ef::row::IRow>::from_row(&[
        DbValue::I32(1),
        DbValue::String("two".to_string()),
        DbValue::I32(3),
    ])
    .expect_err("extra columns are a mapping drift");
    assert!(matches!(too_long, EFError::TypeConversion(..)));
    let message = too_long.to_string();
    assert!(
        message.contains("exactly 2") && message.contains("column 2"),
        "unexpected error: {message}"
    );

    let exact = <TwoColumnRow as rust_ef::row::IRow>::from_row(&[
        DbValue::I32(1),
        DbValue::String("two".to_string()),
    ])
    .expect("exact row");
    assert_eq!(exact.first, 1);
    assert_eq!(exact.second, "two");
}

#[test]
fn entity_positional_mapping_tolerates_trailing_join_columns() {
    // `SELECT * FROM items JOIN ...` appends the joined table's columns; entity
    // materialization maps the leading entity columns and ignores the tail.
    let entity = RowItem::from_row(&[
        DbValue::I32(1),
        DbValue::String("books".to_string()),
        DbValue::I64(10),
        DbValue::I64(999),
    ])
    .expect("entity rows may carry trailing columns");
    assert_eq!(entity.id, 1);
    assert_eq!(entity.category, "books");
    assert_eq!(entity.amount, 10);
}

#[test]
fn report_row_mappings_are_width_strict() {
    let too_wide = CategoryTotal::from_row(&[
        DbValue::String("books".to_string()),
        DbValue::I64(150),
        DbValue::I64(0),
    ]);
    assert!(
        too_wide.is_err(),
        "report structs must reject extra columns"
    );

    let too_wide_tuple =
        <(i32, i64)>::from_row(&[DbValue::I32(1), DbValue::I64(2), DbValue::I64(3)]);
    assert!(too_wide_tuple.is_err(), "tuples must reject extra columns");
}
