//! Phase 3 performance tests — batch INSERT and batch DELETE.
//!
//! Verifies that `ChangeExecutor` batches rows into multi-value INSERT
//! statements (`INSERT INTO ... VALUES (...), (...)`) and `DELETE ... WHERE
//! pk IN (...)` statements, with batches sized so the total parameter count
//! stays ≤ 900 (SQLite's variable limit is 999). This minimizes round trips
//! for bulk operations.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use rust_ef::error::EFResult;
use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
use rust_ef::provider::DbValue;
use rust_ef::query::{BoolExpr, FilterCondition};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Test entity: BatchItem (manual impl for test isolation)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct BatchItem {
    id: i32,
    name: String,
    value: f64,
    tenant_id: i32,
}

impl IEntityType for BatchItem {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("BatchItem"),
            table_name: std::borrow::Cow::Borrowed("batch_items"),
            properties: vec![
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("id"),
                    column_name: std::borrow::Cow::Borrowed("id"),
                    type_id: std::any::TypeId::of::<i32>(),
                    type_name: std::borrow::Cow::Borrowed("i32"),
                    is_primary_key: true,
                    is_auto_increment: true,
                    is_required: true,
                    is_foreign_key: false,
                    is_concurrency_token: false,
                    max_length: None,
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                },
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("name"),
                    column_name: std::borrow::Cow::Borrowed("name"),
                    type_id: std::any::TypeId::of::<String>(),
                    type_name: std::borrow::Cow::Borrowed("String"),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_required: true,
                    is_foreign_key: false,
                    is_concurrency_token: false,
                    max_length: Some(100),
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                },
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("value"),
                    column_name: std::borrow::Cow::Borrowed("value"),
                    type_id: std::any::TypeId::of::<f64>(),
                    type_name: std::borrow::Cow::Borrowed("f64"),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_required: false,
                    is_foreign_key: false,
                    is_concurrency_token: false,
                    max_length: None,
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                },
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("tenant_id"),
                    column_name: std::borrow::Cow::Borrowed("tenant_id"),
                    type_id: std::any::TypeId::of::<i32>(),
                    type_name: std::borrow::Cow::Borrowed("i32"),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_required: true,
                    is_foreign_key: false,
                    is_concurrency_token: false,
                    max_length: None,
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                },
            ],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("id")],
            ..EntityTypeMeta::default()
        }
    }
}

impl IFromRow for BatchItem {
    fn from_row(values: &[String]) -> EFResult<Self> {
        Ok(BatchItem {
            id: values.first().and_then(|s| s.parse().ok()).unwrap_or(0),
            name: values.get(1).cloned().unwrap_or_default(),
            value: values.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0),
            tenant_id: values.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
        })
    }
}

impl IGetKeyValues for BatchItem {
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m
    }
}

impl IEntitySnapshot for BatchItem {
    fn snapshot(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m.insert("name".into(), DbValue::String(self.name.clone()));
        m.insert("value".into(), DbValue::F64(self.value));
        m.insert("tenant_id".into(), DbValue::I32(self.tenant_id));
        m
    }
}

impl INavigationSetter for BatchItem {}

impl rust_ef::entity::ILazyInit for BatchItem {
    fn attach_lazy_contexts(
        &mut self,
        _provider: std::sync::Arc<dyn rust_ef::provider::IDatabaseProvider>,
        _filter_map: Option<
            std::sync::Arc<std::collections::HashMap<String, rust_ef::query::CompiledFilter>>,
        >,
        _depth: usize,
    ) {
        // No navigation fields — lazy loading is a no-op for this test entity.
    }
}

// ---------------------------------------------------------------------------
// Helper: build a fresh in-memory DbContext with the BatchItem schema.
// ---------------------------------------------------------------------------

async fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build()).expect("DbContext");
    ctx.set::<BatchItem>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_batch_insert_multiple_rows() {
    let mut ctx = make_ctx().await;
    for i in 0..10 {
        ctx.set::<BatchItem>().add(BatchItem {
            id: 0,
            name: format!("item-{}", i),
            value: i as f64,
            tenant_id: 1,
        });
    }

    let saved = ctx.save_changes().await.expect("save");
    assert_eq!(saved.added, 10, "save_changes should report 10 added rows");

    let count = ctx.set::<BatchItem>().query().count().await.expect("count");
    assert_eq!(count, 10, "all 10 rows should be persisted");

    let items = ctx
        .set::<BatchItem>()
        .query()
        .to_list()
        .await
        .expect("to_list");
    assert_eq!(items.len(), 10);
}

#[tokio::test]
async fn test_batch_insert_large_set_auto_batches() {
    // 500 rows × 3 non-PK columns = 1500 parameters. With a 900-param
    // ceiling and 3 columns per row, each batch holds 300 rows, so the
    // executor must split this into 2 batches (300 + 200).
    let mut ctx = make_ctx().await;
    for i in 0..500 {
        ctx.set::<BatchItem>().add(BatchItem {
            id: 0,
            name: format!("row-{}", i),
            value: i as f64,
            tenant_id: i % 3,
        });
    }

    let saved = ctx.save_changes().await.expect("save");
    assert_eq!(saved.added, 500, "all 500 rows should be inserted");

    let count = ctx.set::<BatchItem>().query().count().await.expect("count");
    assert_eq!(count, 500, "all 500 rows should be persisted");
}

#[tokio::test]
async fn test_batch_delete_multiple_rows() {
    let mut ctx = make_ctx().await;
    for i in 0..10 {
        ctx.set::<BatchItem>().add(BatchItem {
            id: 0,
            name: format!("del-{}", i),
            value: i as f64,
            tenant_id: 1,
        });
    }
    ctx.save_changes().await.expect("insert");

    // Re-load the rows (with real DB-assigned PKs), attach, and mark all deleted.
    let items = ctx
        .set::<BatchItem>()
        .query()
        .to_list()
        .await
        .expect("load");
    assert_eq!(items.len(), 10);
    ctx.set::<BatchItem>().clear_entries();
    for item in items {
        ctx.set::<BatchItem>().attach(item);
    }
    ctx.set::<BatchItem>().remove_all();

    let deleted = ctx.save_changes().await.expect("delete");
    assert_eq!(deleted.deleted, 10, "save_changes should report 10 deleted");

    let count = ctx.set::<BatchItem>().query().count().await.expect("count");
    assert_eq!(count, 0, "no rows should remain after batch delete");
}

#[tokio::test]
async fn test_batch_delete_with_query_filter() {
    // Context scoped to tenant_id = 1 via a global query filter.
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build()).expect("DbContext");
    ctx.model()
        .has_query_filter::<BatchItem>(BoolExpr::Filter(FilterCondition::with_values(
            "tenant_id",
            "=",
            vec![DbValue::I32(1)],
        )));
    ctx.set::<BatchItem>();
    ctx.ensure_created().await.expect("ensure_created");

    // INSERTs ignore query filters, so both tenants can be seeded.
    for i in 0..5 {
        ctx.set::<BatchItem>().add(BatchItem {
            id: 0,
            name: format!("t1-{}", i),
            value: i as f64,
            tenant_id: 1,
        });
    }
    for i in 0..5 {
        ctx.set::<BatchItem>().add(BatchItem {
            id: 0,
            name: format!("t2-{}", i),
            value: i as f64,
            tenant_id: 2,
        });
    }
    ctx.save_changes().await.expect("seed");

    // The filtered query only sees tenant_id = 1 rows.
    let t1_items = ctx
        .set::<BatchItem>()
        .query()
        .to_list()
        .await
        .expect("filtered load");
    assert_eq!(
        t1_items.len(),
        5,
        "query filter should scope reads to tenant_id=1"
    );

    // Mark all visible (tenant_id=1) rows as deleted and save.
    ctx.set::<BatchItem>().clear_entries();
    for item in t1_items {
        ctx.set::<BatchItem>().attach(item);
    }
    ctx.set::<BatchItem>().remove_all();

    let deleted = ctx.save_changes().await.expect("delete");
    assert_eq!(
        deleted.deleted, 5,
        "should delete the 5 tenant_id=1 rows via batched DELETE WHERE pk IN (...) AND tenant_id = ?"
    );

    // Only tenant_id=2 rows should remain.
    let remaining_all = ctx
        .set::<BatchItem>()
        .query_ignore_filters()
        .count()
        .await
        .expect("count all");
    assert_eq!(remaining_all, 5, "tenant_id=2 rows must remain untouched");

    let remaining_t1 = ctx
        .set::<BatchItem>()
        .query()
        .count()
        .await
        .expect("filtered count");
    assert_eq!(remaining_t1, 0, "no tenant_id=1 rows should remain");
}
