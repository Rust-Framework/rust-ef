//! Navigation performance tests — String PK HasMany include.
//!
//! Verifies the De-String materialization pipeline correctly handles String
//! primary keys. `db_value_key` produces quoted output for `DbValue::String`
//! (e.g. `'p1'`), so parent PK and child FK keys must both go through
//! `db_value_key` to match. Before De-String, `group_rows` used bare String
//! while `db_value_key` produced quoted output, causing String PK include to
//! silently return empty children.
//!
//! Entities use manual `IEntityType` / `IFromRow` / `INavigationSetter`
//! impls (no derive macro) for test isolation.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use rust_ef::entity_snapshot::EntitySnapshot;
use rust_ef::error::EFResult;
use rust_ef::metadata::{EntityTypeMeta, NavigationKind, NavigationMeta, PropertyMeta};
use rust_ef::provider::DbValue;
use rust_ef::relations::HasMany;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

// ---------------------------------------------------------------------------
// StringPkParent — String PK with HasMany<StringPkChild> navigation.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct StringPkParent {
    code: String,
    name: String,
    children: HasMany<StringPkChild>,
}

impl IEntityType for StringPkParent {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("StringPkParent"),
            table_name: std::borrow::Cow::Borrowed("string_pk_parents"),
            properties: vec![
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("code"),
                    column_name: std::borrow::Cow::Borrowed("code"),
                    type_id: std::any::TypeId::of::<String>(),
                    type_name: std::borrow::Cow::Borrowed("String"),
                    is_primary_key: true,
                    is_auto_increment: false,
                    is_sequence: false,
                    sequence_name: None,
                    is_required: true,
                    is_foreign_key: false,
                    is_concurrency_token: false,
                    max_length: Some(50),
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
            ],
            navigations: vec![NavigationMeta {
                field_name: std::borrow::Cow::Borrowed("children"),
                kind: NavigationKind::HasMany,
                related_type_id: std::any::TypeId::of::<StringPkChild>(),
                related_type_name: std::borrow::Cow::Borrowed("StringPkChild"),
                foreign_key_field: None,
                inverse_navigation: None,
                through_type_id: None,
                through_table: None,
                through_parent_fk: None,
                through_related_fk: None,
                through_parent_fk_index: 0,
                through_related_fk_index: 0,
                related_table: Some(std::borrow::Cow::Borrowed("string_pk_children")),
                fk_column: Some(std::borrow::Cow::Borrowed("parent_code")),
                referenced_key_column: Some(std::borrow::Cow::Borrowed("code")),
                // StringPkChild row layout: [id, parent_code, label] → parent_code at index 1.
                fk_row_index: 1,
                pk_row_index: 0,
                related_entity_meta: Some(StringPkChild::entity_meta),
                delete_behavior: None,
            }],
            primary_keys: vec![std::borrow::Cow::Borrowed("code")],
            ..EntityTypeMeta::default()
        }
    }
}

impl IFromRow for StringPkParent {
    fn from_row(values: &[DbValue]) -> EFResult<Self> {
        Ok(StringPkParent {
            code: values
                .first()
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
            name: values
                .get(1)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
            children: HasMany::new(),
        })
    }
}

impl IGetKeyValues for StringPkParent {
    fn key_values(&self) -> EntitySnapshot {
        EntitySnapshot::new(vec![("code", DbValue::String(self.code.clone()))])
    }
}

impl IEntitySnapshot for StringPkParent {
    fn snapshot(&self) -> EntitySnapshot {
        EntitySnapshot::new(vec![
            ("code", DbValue::String(self.code.clone())),
            ("name", DbValue::String(self.name.clone())),
        ])
    }
}

impl INavigationSetter for StringPkParent {
    fn apply_has_many(&mut self, field: &str, rows: &[Vec<DbValue>]) -> EFResult<()> {
        if field == "children" {
            let items: EFResult<Vec<StringPkChild>> = rows
                .iter()
                .map(|r| <StringPkChild as IFromRow>::from_row(r))
                .collect();
            self.children = HasMany::with(items?);
            return Ok(());
        }
        Ok(())
    }
}

impl rust_ef::entity::ILazyInit for StringPkParent {
    fn attach_lazy_contexts(
        &mut self,
        _provider: std::sync::Arc<dyn rust_ef::provider::IDatabaseProvider>,
        _filter_map: Option<
            std::sync::Arc<std::collections::HashMap<String, rust_ef::query::CompiledFilter>>,
        >,
        _depth: usize,
    ) {
    }
}

// ---------------------------------------------------------------------------
// StringPkChild — dependent side (FK parent_code → StringPkParent.code).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct StringPkChild {
    id: i32,
    parent_code: String,
    label: String,
}

impl IEntityType for StringPkChild {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("StringPkChild"),
            table_name: std::borrow::Cow::Borrowed("string_pk_children"),
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
                    field_name: std::borrow::Cow::Borrowed("parent_code"),
                    column_name: std::borrow::Cow::Borrowed("parent_code"),
                    type_id: std::any::TypeId::of::<String>(),
                    type_name: std::borrow::Cow::Borrowed("String"),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_sequence: false,
                    sequence_name: None,
                    is_required: true,
                    is_foreign_key: true,
                    is_concurrency_token: false,
                    max_length: Some(50),
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                },
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("label"),
                    column_name: std::borrow::Cow::Borrowed("label"),
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
            ],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("id")],
            ..EntityTypeMeta::default()
        }
    }
}

impl IFromRow for StringPkChild {
    fn from_row(values: &[DbValue]) -> EFResult<Self> {
        Ok(StringPkChild {
            id: values
                .first()
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or(0),
            parent_code: values
                .get(1)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
            label: values
                .get(2)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
        })
    }
}

impl IGetKeyValues for StringPkChild {
    fn key_values(&self) -> EntitySnapshot {
        EntitySnapshot::new(vec![("id", DbValue::I32(self.id))])
    }
}

impl IEntitySnapshot for StringPkChild {
    fn snapshot(&self) -> EntitySnapshot {
        EntitySnapshot::new(vec![
            ("id", DbValue::I32(self.id)),
            ("parent_code", DbValue::String(self.parent_code.clone())),
            ("label", DbValue::String(self.label.clone())),
        ])
    }
}

impl INavigationSetter for StringPkChild {}

impl rust_ef::entity::ILazyInit for StringPkChild {
    fn attach_lazy_contexts(
        &mut self,
        _provider: std::sync::Arc<dyn rust_ef::provider::IDatabaseProvider>,
        _filter_map: Option<
            std::sync::Arc<std::collections::HashMap<String, rust_ef::query::CompiledFilter>>,
        >,
        _depth: usize,
    ) {
    }
}

// ---------------------------------------------------------------------------

async fn make_string_pk_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build()).expect("DbContext");
    ctx.set::<StringPkParent>();
    ctx.set::<StringPkChild>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

#[tokio::test]
async fn test_string_pk_has_many_include() {
    let mut ctx = make_string_pk_ctx().await;

    for code in &["p1", "p2", "p3"] {
        ctx.add::<StringPkParent>(StringPkParent {
            code: (*code).to_string(),
            name: format!("parent-{}", code),
            children: HasMany::new(),
        });
    }
    ctx.save_changes().await.expect("insert parents");

    for code in &["p1", "p2", "p3"] {
        for c in 0..2 {
            ctx.add::<StringPkChild>(StringPkChild {
                id: 0,
                parent_code: (*code).to_string(),
                label: format!("{}-{}", code, c),
            });
        }
    }
    ctx.save_changes().await.expect("insert children");

    let loaded = ctx
        .set::<StringPkParent>()
        .query()
        .include_internal("children")
        .to_list()
        .await
        .expect("include query");

    assert_eq!(loaded.len(), 3, "3 parents should be loaded");
    let total_children: usize = loaded.iter().map(|p| p.children.len()).sum();
    assert_eq!(total_children, 6, "all 6 children should be loaded");
    for parent in &loaded {
        assert_eq!(
            parent.children.len(),
            2,
            "each parent should have 2 children"
        );
    }
}
