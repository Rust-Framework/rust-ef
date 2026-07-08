//! Shared CRUD test entity and helpers for multi-provider integration tests.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
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

impl TestItem {
    pub const COLUMN_NAME: &'static str = "name";
    pub const COLUMN_VALUE: &'static str = "value";
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
            ..EntityTypeMeta::default()
        }
    }
}

impl IFromRow for TestItem {
    fn from_row(values: &[DbValue]) -> EFResult<Self> {
        Ok(TestItem {
            id: values.first().and_then(|v| v.clone().try_into().ok()).unwrap_or(0),
            name: values.get(1).and_then(|v| v.clone().try_into().ok()).unwrap_or_default(),
            value: values.get(2).and_then(|v| v.clone().try_into().ok()).unwrap_or(0.0),
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

impl rust_ef::entity::ILazyInit for TestItem {
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

/// linq! filter + IS NULL / IS NOT NULL (SQLite scenario 2).
pub async fn run_filter_with_in_operator(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    use rust_ef::linq;
    reset_schema(&*provider, dialect).await?;
    let mut ctx = db_context_with_provider(provider);
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "A".into(),
        value: 1.0,
    });
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "B".into(),
        value: 2.0,
    });
    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "C".into(),
        value: 3.0,
    });
    ctx.save_changes().await?;

    let items = ctx
        .set::<TestItem>()
        .filter(linq!(TestItem, |t| t.value > 1.5))
        .to_list()
        .await?;
    assert_eq!(items.len(), 2);

    let items_null = ctx
        .set::<TestItem>()
        .filter(linq!(TestItem, |t| t.value.is_null()))
        .to_list()
        .await?;
    assert_eq!(items_null.len(), 0);

    let items_not_null = ctx
        .set::<TestItem>()
        .filter(linq!(TestItem, |t| t.name.is_not_null()))
        .to_list()
        .await?;
    assert_eq!(items_not_null.len(), 3);
    Ok(())
}

/// Pagination take/skip (SQLite scenario 3).
pub async fn run_limit_and_offset(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    reset_schema(&*provider, dialect).await?;
    let mut ctx = db_context_with_provider(provider);
    for i in 0..10 {
        ctx.set::<TestItem>().add(TestItem {
            id: 0,
            name: format!("Item{}", i),
            value: i as f64,
        });
    }
    ctx.save_changes().await?;

    let items = ctx.set::<TestItem>().query().take(3).to_list().await?;
    assert_eq!(items.len(), 3);

    let items = ctx
        .set::<TestItem>()
        .query()
        .skip(8)
        .take(5)
        .to_list()
        .await?;
    assert_eq!(items.len(), 2);
    Ok(())
}

/// count + any existence checks (SQLite scenario 4).
pub async fn run_count_and_any(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    use rust_ef::linq;
    reset_schema(&*provider, dialect).await?;
    let mut ctx = db_context_with_provider(provider);
    for i in 0..5 {
        ctx.set::<TestItem>().add(TestItem {
            id: 0,
            name: "X".into(),
            value: i as f64,
        });
    }
    ctx.save_changes().await?;

    let count = ctx.set::<TestItem>().query().count().await?;
    assert_eq!(count, 5);

    let any = ctx
        .set::<TestItem>()
        .filter(linq!(TestItem, |t| t.value == 3))
        .any()
        .await?;
    assert!(any);

    let any_none = ctx
        .set::<TestItem>()
        .filter(linq!(TestItem, |t| t.value == 99))
        .any()
        .await?;
    assert!(!any_none);
    Ok(())
}

/// Aggregation sum/avg (SQLite scenario 6).
pub async fn run_aggregation_queries(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    use rust_ef::linq;
    reset_schema(&*provider, dialect).await?;
    let mut ctx = db_context_with_provider(provider);
    for i in 1..=5 {
        ctx.set::<TestItem>().add(TestItem {
            id: 0,
            name: "Agg".into(),
            value: i as f64,
        });
    }
    ctx.save_changes().await?;

    let sum = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; sum b.value).await?;
    assert!((sum - 15.0).abs() < 0.01, "sum should be 15.0, got {}", sum);

    let avg = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; avg b.value).await?;
    assert!((avg - 3.0).abs() < 0.01, "avg should be 3.0, got {}", avg);
    Ok(())
}

/// Empty table query returns [] / count 0 / any false / first_or_default None (SQLite scenario 7).
pub async fn run_empty_result_handling(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    reset_schema(&*provider, dialect).await?;
    let mut ctx = db_context_with_provider(provider);

    let items = ctx.set::<TestItem>().query().to_list().await?;
    assert!(items.is_empty());

    let count = ctx.set::<TestItem>().query().count().await?;
    assert_eq!(count, 0);

    let any = ctx.set::<TestItem>().query().any().await?;
    assert!(!any);

    let first = ctx.set::<TestItem>().query().first_or_default().await?;
    assert!(first.is_none());
    Ok(())
}

/// ensure_created → insert → ensure_deleted → ensure_created resets (SQLite scenario 8).
pub async fn run_ensure_created_and_deleted(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    // Start from a clean slate: ensure_deleted then ensure_created.
    let meta = TestItem::entity_meta();
    let engine = MigrationEngine::new(dialect);
    let _ = engine
        .ensure_deleted(&*provider, std::slice::from_ref(&meta))
        .await;

    let mut ctx = db_context_with_provider(provider);
    ctx.set::<TestItem>();
    ctx.ensure_created().await?;

    ctx.set::<TestItem>().add(TestItem {
        id: 0,
        name: "Created".into(),
        value: 1.0,
    });
    let result = ctx.save_changes().await?;
    assert_eq!(result.added, 1);

    let items = ctx.set::<TestItem>().query().to_list().await?;
    assert_eq!(items.len(), 1);

    ctx.ensure_deleted().await?;
    ctx.ensure_created().await?;
    let items = ctx.set::<TestItem>().query().to_list().await?;
    assert!(items.is_empty());
    Ok(())
}

/// has_data seed rows materialized on ensure_created (SQLite scenario 9).
pub async fn run_has_data_seed(
    provider: Arc<dyn IDatabaseProvider>,
    dialect: MigrationDialect,
) -> EFResult<()> {
    let meta = TestItem::entity_meta();
    let engine = MigrationEngine::new(dialect);
    let _ = engine
        .ensure_deleted(&*provider, std::slice::from_ref(&meta))
        .await;

    let mut ctx = db_context_with_provider(provider);
    ctx.model().entity::<TestItem>().has_data(&[TestItem {
        id: 1,
        name: "Seed".into(),
        value: 9.0,
    }]);
    ctx.set::<TestItem>();
    ctx.ensure_created().await?;

    let items = ctx.set::<TestItem>().query().to_list().await?;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "Seed");
    assert!((items[0].value - 9.0).abs() < f64::EPSILON);
    Ok(())
}
