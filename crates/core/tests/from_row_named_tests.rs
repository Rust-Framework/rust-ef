//! EF-6: name-based row materialization (`IFromRow::from_row_named`).
//!
//! The positional `from_row(&[DbValue])` assigns by index, so a row whose
//! column order differs from the model order (a migration that reordered
//! columns, a view, an ad-hoc `SELECT`) silently shifts values into the wrong
//! fields. `from_row_named` matches by column name and is the reorder-safe
//! path; the positional API stays for compatibility.

mod common;

use rust_ef::entity::IFromRow;
use rust_ef::error::EFError;
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;

#[derive(Debug, Clone, EntityType)]
#[table("ef6_plain")]
struct PlainItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    /// Deliberately the same Rust type as `b` so a positional swap is silent.
    a: i32,
    b: i32,
}

/// View entity: column order in a view is not guaranteed to follow the model.
#[derive(Debug, Clone, EntityType)]
#[table("v_ef6_summary", view)]
struct ItemSummaryView {
    a: i32,
    b: i32,
}

/// Column name (`legacy_code`) differs from the field name (`code`).
#[derive(Debug, Clone, EntityType)]
#[table("ef6_renamed")]
struct RenamedItem {
    #[primary_key]
    id: i32,
    #[column("legacy_code")]
    #[required]
    code: String,
    a: i32,
}

fn named(id: i32, a: i32, b: i32) -> Vec<(&'static str, DbValue)> {
    // Shuffled order: the model order is (id, a, b).
    vec![
        ("b", DbValue::I32(b)),
        ("id", DbValue::I32(id)),
        ("a", DbValue::I32(a)),
    ]
}

#[test]
fn positional_from_row_is_order_sensitive() {
    // Documented hazard: same-type columns swap silently on reordered rows.
    let positional = PlainItem::from_row(&[DbValue::I32(2), DbValue::I32(1), DbValue::I32(99)])
        .expect("positional row");
    assert_eq!(
        (positional.id, positional.a, positional.b),
        (2, 1, 99),
        "positional materialization assigns by index"
    );
}

#[test]
fn named_from_row_restores_the_right_fields() {
    let item = PlainItem::from_row_named(&named(99, 1, 2)).expect("named row");
    assert_eq!((item.id, item.a, item.b), (99, 1, 2));
}

#[test]
fn named_from_row_supports_views_and_extra_values() {
    // A view may expose extra computed columns; unused values with no name are
    // harmless because matching is by name.
    let mut columns = named(7, 10, 20);
    columns.push(("computed_total", DbValue::I64(1_000)));

    let view = ItemSummaryView::from_row_named(&columns).expect("view row");
    assert_eq!((view.a, view.b), (10, 20));
}

#[test]
fn named_from_row_matches_the_column_name_not_the_field_name() {
    let columns = vec![
        ("a", DbValue::I32(5)),
        ("legacy_code", DbValue::String("ABC".into())),
        ("id", DbValue::I32(3)),
    ];
    let item = RenamedItem::from_row_named(&columns).expect("renamed row");
    assert_eq!(item.code, "ABC");
    assert_eq!((item.id, item.a), (3, 5));
}

#[test]
fn named_from_row_reports_missing_columns_typed() {
    // `b` is missing from the row.
    let columns = vec![("a", DbValue::I32(1)), ("id", DbValue::I32(2))];
    let err = PlainItem::from_row_named(&columns).expect_err("missing column must fail");
    match err {
        EFError::TypeConversion(msg, _) => {
            assert!(msg.contains("column `b`"), "unexpected message: {msg}");
            assert!(msg.contains("PlainItem"), "unexpected message: {msg}");
        }
        other => panic!("expected TypeConversion, got {other:?}"),
    }
}

#[test]
fn materialize_entities_named_handles_reordered_rows() {
    let columns = ["b", "id", "a"];
    let rows = vec![
        vec![DbValue::I32(2), DbValue::I32(99), DbValue::I32(1)],
        vec![DbValue::I32(4), DbValue::I32(100), DbValue::I32(3)],
    ];

    let items = rust_ef::entity::materialize_entities_named::<PlainItem>(&columns, &rows)
        .expect("named materialization");
    assert_eq!(items.len(), 2);
    assert_eq!((items[0].id, items[0].a, items[0].b), (99, 1, 2));
    assert_eq!((items[1].id, items[1].a, items[1].b), (100, 3, 4));
}

#[test]
fn materialize_entities_named_rejects_short_rows() {
    let columns = ["id", "a", "b"];
    let rows = vec![vec![DbValue::I32(1), DbValue::I32(2)]];

    let err = rust_ef::entity::materialize_entities_named::<PlainItem>(&columns, &rows)
        .expect_err("short row must fail");
    assert!(matches!(err, EFError::TypeConversion(..)), "got {err:?}");
}

/// Manual `IFromRow` implementations (no derive) use the default
/// `from_row_named`, which reorders via `entity_meta()` property order.
#[test]
fn default_named_materialization_works_for_manual_impls() {
    // Model/positional order is (id, name, value) for `common::TestItem`.
    let columns = vec![
        ("value", DbValue::F64(12.5)),
        ("name", DbValue::String("named".into())),
        ("id", DbValue::I32(42)),
    ];
    let item = common::TestItem::from_row_named(&columns).expect("manual default path");
    assert_eq!(item.id, 42);
    assert_eq!(item.name, "named");
    assert_eq!(item.value, 12.5);
}
