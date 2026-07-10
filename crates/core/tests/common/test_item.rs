//! Shared `TestItem` entity for multi-provider integration tests.

#![allow(dead_code)]

use rust_ef::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use rust_ef::error::EFResult;
use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
use rust_ef::provider::DbValue;
use std::collections::HashMap;

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
                    is_sequence: false,
                    sequence_name: None,
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
                    is_sequence: false,
                    sequence_name: None,
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
                    is_sequence: false,
                    sequence_name: None,
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
            id: values
                .first()
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or(0),
            name: values
                .get(1)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
            value: values
                .get(2)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or(0.0),
        })
    }
}

impl IGetKeyValues for TestItem {
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m
    }
    fn set_auto_increment_key(&mut self, key: i64) {
        self.id = key as i32;
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
