//! SQLite integration tests ??full CRUD lifecycle with real database.
//!
//! These tests verify end-to-end correctness of the ORM with
//! an actual SQLite in-memory database.

#[cfg(test)]
mod sqlite_crud {
    use rust_ef::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues};
    use rust_ef::entity_snapshot::EntitySnapshot;
    use rust_ef::error::EFResult;
    use rust_ef::linq;
    use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
    use rust_ef::provider::DbValue;

    fn make_ctx() -> rust_ef::db_context::DbContext {
        use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
        use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        DbContext::from_options(&builder.build()).expect("ctx")
    }

    // -----------------------------------------------------------------------
    // Test entity: a minimal entity without derive (manual impl for test isolation)
    // -----------------------------------------------------------------------

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
        fn key_values(&self) -> EntitySnapshot {
            EntitySnapshot::new(vec![("id", DbValue::I32(self.id))])
        }
    }

    impl IEntitySnapshot for TestItem {
        fn snapshot(&self) -> EntitySnapshot {
            EntitySnapshot::new(vec![
                ("id", DbValue::I32(self.id)),
                ("name", DbValue::String(self.name.clone())),
                ("value", DbValue::F64(self.value)),
            ])
        }
    }

    impl rust_ef::entity::INavigationSetter for TestItem {}

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

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_insert_and_query() {
        let mut ctx = make_ctx();
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "Alpha".into(),
            value: 1.0,
        });
        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "Beta".into(),
            value: 2.0,
        });

        let result = ctx.save_changes().await.expect("save");
        assert_eq!(result.added, 2);

        ctx.set::<TestItem>().clear_entries();
        let items = ctx
            .set::<TestItem>()
            .query()
            .to_list()
            .await
            .expect("query to_list");
        assert_eq!(items.len(), 2);

        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
    }

    #[tokio::test]
    async fn test_filter_with_in_operator() {
        let mut ctx = make_ctx();
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "A".into(),
            value: 1.0,
        });
        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "B".into(),
            value: 2.0,
        });
        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "C".into(),
            value: 3.0,
        });
        ctx.save_changes().await.unwrap();
        ctx.set::<TestItem>().clear_entries();

        // Query with linq filter
        let items = ctx
            .set::<TestItem>()
            .filter(linq!(TestItem, |t| t.value > 1.5))
            .to_list()
            .await
            .unwrap();
        assert_eq!(items.len(), 2);

        // Query with IS NULL via linq
        let items_null = ctx
            .set::<TestItem>()
            .filter(linq!(TestItem, |t| t.value.is_null()))
            .to_list()
            .await
            .unwrap();
        assert_eq!(items_null.len(), 0);

        // Query with IS NOT NULL via linq
        let items_not_null = ctx
            .set::<TestItem>()
            .filter(linq!(TestItem, |t| t.name.is_not_null()))
            .to_list()
            .await
            .unwrap();
        assert_eq!(items_not_null.len(), 3);
    }

    #[tokio::test]
    async fn test_limit_and_offset() {
        let mut ctx = make_ctx();
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        for i in 0..10 {
            ctx.add::<TestItem>(TestItem {
                id: 0,
                name: format!("Item{}", i),
                value: i as f64,
            });
        }
        ctx.save_changes().await.unwrap();
        ctx.set::<TestItem>().clear_entries();

        // Test take
        let items = ctx
            .set::<TestItem>()
            .query()
            .take(3)
            .to_list()
            .await
            .unwrap();
        assert_eq!(items.len(), 3);

        // Test skip + take
        let items = ctx
            .set::<TestItem>()
            .query()
            .skip(8)
            .take(5)
            .to_list()
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_count_and_any() {
        let mut ctx = make_ctx();
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        for i in 0..5 {
            ctx.add::<TestItem>(TestItem {
                id: 0,
                name: "X".into(),
                value: i as f64,
            });
        }
        ctx.save_changes().await.unwrap();
        ctx.set::<TestItem>().clear_entries();

        let count = ctx.set::<TestItem>().query().count().await.unwrap();
        assert_eq!(count, 5);

        let any = ctx
            .set::<TestItem>()
            .filter(linq!(TestItem, |t| t.value == 3))
            .any()
            .await
            .unwrap();
        assert!(any);

        let any_none = ctx
            .set::<TestItem>()
            .filter(linq!(TestItem, |t| t.value == 99))
            .any()
            .await
            .unwrap();
        assert!(!any_none);
    }

    #[tokio::test]
    async fn test_update_and_delete() {
        let mut ctx = make_ctx();
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "ToUpdate".into(),
            value: 10.0,
        });
        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "ToDelete".into(),
            value: 20.0,
        });
        ctx.save_changes().await.unwrap();
        ctx.set::<TestItem>().clear_entries();

        // Load from DB, modify, and update
        let items = ctx.set::<TestItem>().query().to_list().await.unwrap();
        let mut item_to_update = items
            .iter()
            .find(|i| i.name == "ToUpdate")
            .cloned()
            .unwrap();
        item_to_update.value = 99.0;
        ctx.update::<TestItem>(item_to_update);
        ctx.save_changes().await.expect("update save");

        // Load, attach, and delete
        ctx.set::<TestItem>().clear_entries();
        let items = ctx.set::<TestItem>().query().to_list().await.unwrap();
        let item_to_delete = items
            .iter()
            .find(|i| i.name == "ToDelete")
            .cloned()
            .unwrap();
        ctx.attach::<TestItem>(item_to_delete);
        ctx.remove_at::<TestItem>(0).expect("remove_at");
        ctx.save_changes().await.expect("delete save");

        // Verify: 1 item remains, and it's the updated one
        ctx.set::<TestItem>().clear_entries();
        let items = ctx.set::<TestItem>().query().to_list().await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "ToUpdate");
        assert!(
            (items[0].value - 99.0).abs() < f64::EPSILON,
            "value should be 99.0, got {}",
            items[0].value
        );
    }

    #[tokio::test]
    async fn test_aggregation_queries() {
        let mut ctx = make_ctx();
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        for i in 1..=5 {
            ctx.add::<TestItem>(TestItem {
                id: 0,
                name: "Agg".into(),
                value: i as f64,
            });
        }
        ctx.save_changes().await.unwrap();
        ctx.set::<TestItem>().clear_entries();

        let sum = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; sum b.value)
            .await
            .unwrap();
        assert!((sum - 15.0).abs() < 0.01, "sum should be 15.0, got {}", sum);

        let avg = linq!(ctx.set::<TestItem>().query(), |b: TestItem| true; avg b.value)
            .await
            .unwrap();
        assert!((avg - 3.0).abs() < 0.01, "avg should be 3.0, got {}", avg);
    }

    #[tokio::test]
    async fn test_empty_result_handling() {
        let mut ctx = make_ctx();
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        let items = ctx.set::<TestItem>().query().to_list().await.unwrap();
        assert!(items.is_empty());

        let count = ctx.set::<TestItem>().query().count().await.unwrap();
        assert_eq!(count, 0);

        let any = ctx.set::<TestItem>().query().any().await.unwrap();
        assert!(!any);

        let first = ctx
            .set::<TestItem>()
            .query()
            .first_or_default()
            .await
            .unwrap();
        assert!(first.is_none());
    }

    #[tokio::test]
    async fn test_ensure_created_and_deleted() {
        use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
        use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).expect("create context");
        ctx.set::<TestItem>();

        ctx.ensure_created().await.expect("ensure_created");
        ctx.add::<TestItem>(TestItem {
            id: 0,
            name: "Created".into(),
            value: 1.0,
        });
        let result = ctx.save_changes().await.expect("save");
        assert_eq!(result.added, 1);

        let items = ctx.set::<TestItem>().query().to_list().await.unwrap();
        assert_eq!(items.len(), 1);

        ctx.ensure_deleted().await.expect("ensure_deleted");
        ctx.ensure_created().await.expect("recreate after delete");
        let items = ctx.set::<TestItem>().query().to_list().await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn test_has_data_seed_on_ensure_created() {
        use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
        use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let mut ctx = DbContext::from_options(&builder.build()).expect("create context");

        ctx.model().entity::<TestItem>().has_data(&[TestItem {
            id: 1,
            name: "Seed".into(),
            value: 9.0,
        }]);
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");

        let items = ctx.set::<TestItem>().query().to_list().await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Seed");
        assert!((items[0].value - 9.0).abs() < f64::EPSILON);
    }
}
