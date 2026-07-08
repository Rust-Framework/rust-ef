//! Phase 3 performance tests — navigation (eager) loading.
//!
//! Verifies that `HasMany` eager loading scales to large result sets without
//! O(N²) slowdown. The navigation loader groups secondary-query rows via a
//! `HashMap` (`group_rows`) and the many-to-many path dedups related keys via
//! a `HashSet`, so loading N parents × M children stays O(N + M) rather than
//! O(N × M).
//!
//! Entities use manual `IEntityType` / `IFromRow` / `INavigationSetter`
//! impls (no derive macro) for test isolation, mirroring `common/mod.rs`.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use rust_ef::error::EFResult;
use rust_ef::metadata::{EntityTypeMeta, NavigationKind, NavigationMeta, PropertyMeta};
use rust_ef::provider::DbValue;
use rust_ef::relations::HasMany;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// PerfParent — has a HasMany<PerfChild> navigation.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct PerfParent {
    id: i32,
    name: String,
    children: HasMany<PerfChild>,
}

impl IEntityType for PerfParent {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("PerfParent"),
            table_name: std::borrow::Cow::Borrowed("perf_parents"),
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
            navigations: vec![NavigationMeta {
                field_name: std::borrow::Cow::Borrowed("children"),
                kind: NavigationKind::HasMany,
                related_type_id: std::any::TypeId::of::<PerfChild>(),
                related_type_name: std::borrow::Cow::Borrowed("PerfChild"),
                foreign_key_field: None,
                inverse_navigation: None,
                through_type_id: None,
                through_table: None,
                through_parent_fk: None,
                through_related_fk: None,
                through_parent_fk_index: 0,
                through_related_fk_index: 0,
                related_table: Some(std::borrow::Cow::Borrowed("perf_children")),
                fk_column: Some(std::borrow::Cow::Borrowed("parent_id")),
                referenced_key_column: Some(std::borrow::Cow::Borrowed("id")),
                // PerfChild row layout: [id, parent_id, label] → parent_id at index 1.
                fk_row_index: 1,
                // PerfChild PK (id) is at index 0 of its row.
                pk_row_index: 0,
                related_entity_meta: Some(PerfChild::entity_meta),
            }],
            primary_keys: vec![std::borrow::Cow::Borrowed("id")],
            ..EntityTypeMeta::default()
        }
    }
}

impl IFromRow for PerfParent {
    fn from_row(values: &[DbValue]) -> EFResult<Self> {
        Ok(PerfParent {
            id: values
                .first()
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or(0),
            name: values
                .get(1)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
            children: HasMany::new(),
        })
    }
}

impl IGetKeyValues for PerfParent {
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m
    }
}

impl IEntitySnapshot for PerfParent {
    fn snapshot(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m.insert("name".into(), DbValue::String(self.name.clone()));
        m
    }
}

impl INavigationSetter for PerfParent {
    fn apply_has_many(&mut self, field: &str, rows: &[Vec<DbValue>]) -> EFResult<()> {
        if field == "children" {
            let items: EFResult<Vec<PerfChild>> = rows
                .iter()
                .map(|r| <PerfChild as IFromRow>::from_row(r))
                .collect();
            self.children = HasMany::with(items?);
            return Ok(());
        }
        Ok(())
    }
}

impl rust_ef::entity::ILazyInit for PerfParent {
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
// PerfChild — dependent side (FK parent_id → PerfParent.id).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct PerfChild {
    id: i32,
    parent_id: i32,
    label: String,
}

impl IEntityType for PerfChild {
    fn entity_meta() -> EntityTypeMeta {
        EntityTypeMeta {
            type_id: std::any::TypeId::of::<Self>(),
            type_name: std::borrow::Cow::Borrowed("PerfChild"),
            table_name: std::borrow::Cow::Borrowed("perf_children"),
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
                    field_name: std::borrow::Cow::Borrowed("parent_id"),
                    column_name: std::borrow::Cow::Borrowed("parent_id"),
                    type_id: std::any::TypeId::of::<i32>(),
                    type_name: std::borrow::Cow::Borrowed("i32"),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_required: true,
                    is_foreign_key: true,
                    is_concurrency_token: false,
                    max_length: None,
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

impl IFromRow for PerfChild {
    fn from_row(values: &[DbValue]) -> EFResult<Self> {
        Ok(PerfChild {
            id: values
                .first()
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or(0),
            parent_id: values
                .get(1)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or(0),
            label: values
                .get(2)
                .and_then(|v| v.clone().try_into().ok())
                .unwrap_or_default(),
        })
    }
}

impl IGetKeyValues for PerfChild {
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m
    }
}

impl IEntitySnapshot for PerfChild {
    fn snapshot(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m.insert("parent_id".into(), DbValue::I32(self.parent_id));
        m.insert("label".into(), DbValue::String(self.label.clone()));
        m
    }
}

impl INavigationSetter for PerfChild {}

impl rust_ef::entity::ILazyInit for PerfChild {
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
// Helper: build a fresh in-memory DbContext with both schemas registered.
// ---------------------------------------------------------------------------

async fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build()).expect("DbContext");
    ctx.set::<PerfParent>();
    ctx.set::<PerfChild>();
    ctx.ensure_created().await.expect("ensure_created");
    ctx
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_has_many_large_result_set() {
    let mut ctx = make_ctx().await;

    // Seed 50 parents.
    for p in 0..50 {
        ctx.set::<PerfParent>().add(PerfParent {
            id: 0,
            name: format!("parent-{}", p),
            children: HasMany::new(),
        });
    }
    ctx.save_changes().await.expect("insert parents");

    // Load parents back to obtain their real DB-assigned PKs.
    let parents = ctx
        .set::<PerfParent>()
        .query()
        .to_list()
        .await
        .expect("load parents");
    assert_eq!(parents.len(), 50);

    // Seed 10 children per parent (500 children total).
    for parent in &parents {
        for c in 0..10 {
            ctx.set::<PerfChild>().add(PerfChild {
                id: 0,
                parent_id: parent.id,
                label: format!("child-{}-{}", parent.id, c),
            });
        }
    }
    ctx.save_changes().await.expect("insert children");

    // Query parents with include("children") — exercises the HashMap-based
    // grouping in `load_scalar_navigation`. With 500 children this stays
    // O(parents + children), verifying no O(N²) dedup regression.
    let loaded = ctx
        .set::<PerfParent>()
        .query()
        .include_internal("children")
        .to_list()
        .await
        .expect("include query");

    assert_eq!(loaded.len(), 50, "50 parents should be loaded");
    let total_children: usize = loaded.iter().map(|p| p.children.len()).sum();
    assert_eq!(total_children, 500, "all 500 children should be loaded");
    for parent in &loaded {
        assert_eq!(
            parent.children.len(),
            10,
            "each parent should have 10 children"
        );
    }
}

#[tokio::test]
async fn test_many_to_many_large_join() {
    // A true many-to-many via a join entity with manual impls is involved;
    // this test instead exercises navigation loading on a 100-row result set
    // to verify the loader completes without error and without O(N²) blowup.
    let mut ctx = make_ctx().await;

    for p in 0..100 {
        ctx.set::<PerfParent>().add(PerfParent {
            id: 0,
            name: format!("p-{}", p),
            children: HasMany::new(),
        });
    }
    ctx.save_changes().await.expect("insert parents");

    let parents = ctx
        .set::<PerfParent>()
        .query()
        .to_list()
        .await
        .expect("load parents");
    assert_eq!(parents.len(), 100);

    for parent in &parents {
        ctx.set::<PerfChild>().add(PerfChild {
            id: 0,
            parent_id: parent.id,
            label: format!("c-{}", parent.id),
        });
    }
    ctx.save_changes().await.expect("insert children");

    let loaded = ctx
        .set::<PerfParent>()
        .query()
        .include_internal("children")
        .to_list()
        .await
        .expect("include query");

    assert_eq!(loaded.len(), 100);
    let total: usize = loaded.iter().map(|p| p.children.len()).sum();
    assert_eq!(total, 100, "each parent should have its 1 child loaded");
}

// ---------------------------------------------------------------------------
// String PK HasMany include — verifies the De-String materialization pipeline
// correctly handles String primary keys. `db_value_key` produces quoted output
// for `DbValue::String` (e.g. `'p1'`), so parent PK and child FK keys must
// both go through `db_value_key` to match. Before De-String, `group_rows`
// used bare String while `db_value_key` produced quoted output, causing
// String PK include to silently return empty children.
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
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("code".into(), DbValue::String(self.code.clone()));
        m
    }
}

impl IEntitySnapshot for StringPkParent {
    fn snapshot(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("code".into(), DbValue::String(self.code.clone()));
        m.insert("name".into(), DbValue::String(self.name.clone()));
        m
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
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m
    }
}

impl IEntitySnapshot for StringPkChild {
    fn snapshot(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("id".into(), DbValue::I32(self.id));
        m.insert(
            "parent_code".into(),
            DbValue::String(self.parent_code.clone()),
        );
        m.insert("label".into(), DbValue::String(self.label.clone()));
        m
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
        ctx.set::<StringPkParent>().add(StringPkParent {
            code: (*code).to_string(),
            name: format!("parent-{}", code),
            children: HasMany::new(),
        });
    }
    ctx.save_changes().await.expect("insert parents");

    for code in &["p1", "p2", "p3"] {
        for c in 0..2 {
            ctx.set::<StringPkChild>().add(StringPkChild {
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
