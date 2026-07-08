//! Verifies that query filters are applied to UPDATE/DELETE statements
//! issued by `save_changes()`, preventing cross-tenant writes.
//!
//! INSERTs are intentionally not filtered (tenant_id is set by the user
//! before `add()`). UPDATE/DELETE WHERE clauses are AND-ed with the filter
//! (e.g. `tenant_id = ?`), so rows outside the filter boundary are untouched.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::IEntityType;
use rust_ef::error::EFError;
use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
use rust_ef::provider::DbValue;
use rust_ef::query::{BoolExpr, FilterCondition};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
struct TenantItem {
    id: i32,
    tenant_id: i32,
    name: String,
}

impl IEntityType for TenantItem {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("TenantItem"),
            table_name: std::borrow::Cow::Borrowed("tenant_items"),
            properties: vec![
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("id"),
                    column_name: std::borrow::Cow::Borrowed("id"),
                    type_id: std::any::TypeId::of::<i32>(),
                    type_name: std::borrow::Cow::Borrowed("i32"),
                    is_primary_key: true,
                    is_auto_increment: false,
                    is_required: true,
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
            ],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("id")],
            ..EntityTypeMeta::default()
        }
    }
}

impl rust_ef::entity::IFromRow for TenantItem {
    fn from_row(values: &[DbValue]) -> rust_ef::error::EFResult<Self> {
        Ok(TenantItem {
            id: values.first().and_then(|v| v.clone().try_into().ok()).unwrap_or(0),
            tenant_id: values.get(1).and_then(|v| v.clone().try_into().ok()).unwrap_or(0),
            name: values.get(2).and_then(|v| v.clone().try_into().ok()).unwrap_or_default(),
        })
    }
}

impl rust_ef::entity::IEntitySnapshot for TenantItem {
    fn snapshot(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m.insert("tenant_id".into(), DbValue::I32(self.tenant_id));
        m.insert("name".into(), DbValue::String(self.name.clone()));
        m
    }
}

impl rust_ef::entity::IGetKeyValues for TenantItem {
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m
    }
}

impl rust_ef::entity::INavigationSetter for TenantItem {}

impl rust_ef::entity::ILazyInit for TenantItem {
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

fn tenant_filter(value: i32) -> BoolExpr {
    BoolExpr::Filter(FilterCondition::with_values(
        "tenant_id",
        "=",
        vec![DbValue::I32(value)],
    ))
}

async fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.model().has_query_filter::<TenantItem>(tenant_filter(1));
    ctx.set::<TenantItem>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

/// Helper: insert a row with the given id/tenant_id/name via DbContext.
/// INSERT ignores query filters, so cross-tenant inserts succeed here.
async fn insert_row(ctx: &mut DbContext, id: i32, tenant_id: i32, name: &str) {
    ctx.set::<TenantItem>().add(TenantItem {
        id,
        tenant_id,
        name: name.into(),
    });
    ctx.save_changes().await.expect("insert save");
}

#[tokio::test]
async fn update_across_tenant_filtered_out() {
    let mut ctx = make_ctx().await;
    insert_row(&mut ctx, 10, 2, "other-tenant").await;

    ctx.set::<TenantItem>().update(TenantItem {
        id: 10,
        tenant_id: 2,
        name: "tampered".into(),
    });
    let err = ctx.save_changes().await.unwrap_err();
    match err {
        EFError::ConcurrencyConflict(msg) => {
            assert!(msg.contains("tenant_items"), "msg: {msg}");
        }
        other => panic!("expected ConcurrencyConflict, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_across_tenant_filtered_out() {
    let mut ctx = make_ctx().await;
    insert_row(&mut ctx, 20, 2, "other-tenant").await;

    ctx.set::<TenantItem>().attach(TenantItem {
        id: 20,
        tenant_id: 2,
        name: "other-tenant".into(),
    });
    ctx.set::<TenantItem>().remove_at(0).expect("mark deleted");
    let err = ctx.save_changes().await.unwrap_err();
    match err {
        EFError::ConcurrencyConflict(msg) => {
            assert!(msg.contains("tenant_items"), "msg: {msg}");
        }
        other => panic!("expected ConcurrencyConflict, got {other:?}"),
    }
}

#[tokio::test]
async fn update_within_tenant_succeeds() {
    let mut ctx = make_ctx().await;
    insert_row(&mut ctx, 30, 1, "own-tenant").await;

    ctx.set::<TenantItem>().update(TenantItem {
        id: 30,
        tenant_id: 1,
        name: "renamed".into(),
    });
    let result = ctx.save_changes().await.expect("update should succeed");
    assert_eq!(result.updated, 1, "one row should be updated");
}

#[tokio::test]
async fn query_ignore_filters_returns_all_tenants() {
    let mut ctx = make_ctx().await;
    insert_row(&mut ctx, 40, 1, "own").await;
    insert_row(&mut ctx, 41, 2, "other").await;

    let filtered = ctx
        .set::<TenantItem>()
        .query()
        .to_list()
        .await
        .expect("filtered query");
    assert_eq!(filtered.len(), 1, "query() should only see tenant_id=1");
    assert_eq!(filtered[0].id, 40);

    let all = ctx
        .set::<TenantItem>()
        .query_ignore_filters()
        .to_list()
        .await
        .expect("ignore-filters query");
    assert_eq!(all.len(), 2, "query_ignore_filters() should see all rows");
}
