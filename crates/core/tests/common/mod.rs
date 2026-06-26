//! Shared CRUD test entity and helpers for multi-provider integration tests.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder, IDbContext};
use rust_ef::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use rust_ef::error::EFResult;
use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::provider::{DbValue, IDatabaseProvider};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct TestItem {
    pub id: i32,
    pub name: String,
    pub value: f64,
}

impl IEntityType for TestItem {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("TestItem"),
            table_name: std::borrow::Cow::Borrowed("test_items"),
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
            ],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("id")],
        }
    }
}

impl IFromRow for TestItem {
    fn from_row(values: &[String]) -> EFResult<Self> {
        Ok(TestItem {
            id: values.first().and_then(|s| s.parse().ok()).unwrap_or(0),
            name: values.get(1).cloned().unwrap_or_default(),
            value: values.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0),
        })
    }
}

impl IGetKeyValues for TestItem {
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m
    }
}

impl IEntitySnapshot for TestItem {
    fn snapshot(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m.insert("name".into(), DbValue::String(self.name.clone()));
        m.insert("value".into(), DbValue::F64(self.value));
        m
    }
}

impl INavigationSetter for TestItem {}

#[allow(clippy::type_complexity)]
pub fn db_context_with_provider(provider: Arc<dyn IDatabaseProvider>) -> DbContext {
    let p = provider.clone();
    let factory: Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync> =
        Arc::new(move |_| Ok(p.clone()));
    let mut builder = DbContextOptionsBuilder::new();
    builder.connection_string("integration-test");
    builder.set_provider_factory("integration", "integration-test", factory);
    DbContext::from_options(&builder.build()).expect("DbContext")
}

pub async fn reset_schema(
    provider: &dyn IDatabaseProvider,
    dialect: MigrationDialect,
) -> EFResult<()> {
    let meta = TestItem::entity_meta();
    let engine = MigrationEngine::new(dialect);
    let _ = engine
        .ensure_deleted(provider, std::slice::from_ref(&meta))
        .await;
    engine.ensure_created(provider, &[meta]).await
}

/// Full insert → query → update → delete lifecycle via `DbContext::save_changes`.
pub async fn run_crud_lifecycle(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    reset_schema(&*provider, dialect).await?;

    let mut ctx = db_context_with_provider(provider);
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Alpha".into(),
        value: 1.0,
    });
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Beta".into(),
        value: 2.0,
    });
    let saved = ctx.save_changes().await?;
    assert_eq!(saved.added, 2);

    let items = ctx.set::<TestItem>().query().to_list().await?;
    assert_eq!(items.len(), 2);

    let one = items
        .into_iter()
        .find(|i| i.name == "Alpha")
        .expect("Alpha row");
    ctx.set::<TestItem>().clear_entries();
    ctx.set::<TestItem>().attach(one);
    ctx.set::<TestItem>()
        .tracked_entries_mut()
        .next()
        .unwrap()
        .name = "AlphaUpdated".into();
    ctx.set::<TestItem>().detect_changes();
    let updated = ctx.save_changes().await?;
    assert_eq!(updated.updated, 1);

    let after_update = ctx.set::<TestItem>().query().to_list().await?;
    assert!(after_update.iter().any(|i| i.name == "AlphaUpdated"));

    ctx.set::<TestItem>().clear_entries();
    ctx.set::<TestItem>().attach(
        after_update
            .into_iter()
            .find(|i| i.name == "Beta")
            .expect("Beta row"),
    );
    ctx.set::<TestItem>().remove_at(0).unwrap();
    let deleted = ctx.save_changes().await?;
    assert_eq!(deleted.deleted, 1);
    assert_eq!(ctx.set::<TestItem>().query().count().await?, 1);

    Ok(())
}
