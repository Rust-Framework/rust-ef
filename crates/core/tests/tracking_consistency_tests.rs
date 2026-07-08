//! Verifies that the interceptor `SaveChangesContext` is built from the real
//! pending entries (`DbSet.entries`) rather than the legacy empty
//! `change_tracker`.
//!
//! Before Task 2, `on_saving` received 0 entries even when N were pending.
//! After Task 2 it reflects the actual pending count.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::IEntityType;
use rust_ef::error::EFResult;
use rust_ef::interceptor::{ISaveChangesInterceptor, SaveChangesContext};
use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
struct Item {
    id: i32,
    name: String,
}

impl IEntityType for Item {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("Item"),
            table_name: std::borrow::Cow::Borrowed("items"),
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
            ],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("id")],
            ..EntityTypeMeta::default()
        }
    }
}

impl rust_ef::entity::IFromRow for Item {
    fn from_row(values: &[rust_ef::provider::DbValue]) -> EFResult<Self> {
        Ok(Item {
            id: values
                .first()
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or(0),
            name: values
                .get(1)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
        })
    }
}

impl rust_ef::entity::IEntitySnapshot for Item {
    fn snapshot(&self) -> std::collections::HashMap<String, rust_ef::provider::DbValue> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".into(), rust_ef::provider::DbValue::I32(self.id));
        m.insert(
            "name".into(),
            rust_ef::provider::DbValue::String(self.name.clone()),
        );
        m
    }
}

impl rust_ef::entity::IGetKeyValues for Item {
    fn key_values(&self) -> std::collections::HashMap<String, rust_ef::provider::DbValue> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".into(), rust_ef::provider::DbValue::I32(self.id));
        m
    }
}

impl rust_ef::entity::INavigationSetter for Item {}

impl rust_ef::entity::ILazyInit for Item {
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

/// Captures the `SaveChangesContext` snapshot seen by `on_saving`.
struct CapturingInterceptor {
    seen: Arc<Mutex<Option<(usize, usize, usize)>>>,
}

#[async_trait::async_trait]
impl ISaveChangesInterceptor for CapturingInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        *self.seen.lock().unwrap() =
            Some((ctx.entries().len(), ctx.added_count(), ctx.total_count()));
        Ok(())
    }
}

#[tokio::test]
async fn interceptor_sees_pending_entries_from_dbset() {
    let seen = Arc::new(Mutex::new(None));
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    builder.add_interceptor(CapturingInterceptor { seen: seen.clone() });
    let options = builder.build();

    let mut ctx = DbContext::from_options(&options).expect("ctx");
    ctx.set::<Item>();
    ctx.ensure_created().await.expect("ensure_created");

    // Before save, the interceptor has seen nothing.
    assert!(seen.lock().unwrap().is_none());

    ctx.set::<Item>().add(Item {
        id: 0,
        name: "alpha".into(),
    });
    ctx.set::<Item>().add(Item {
        id: 0,
        name: "beta".into(),
    });
    ctx.save_changes().await.expect("save");

    let captured = seen.lock().unwrap().take().expect("on_saving ran");
    // Two added entries must be visible to the interceptor (previously 0).
    assert_eq!(captured, (2, 2, 2), "interceptor must see pending entries");
}
