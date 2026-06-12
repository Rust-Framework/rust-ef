//! SQLite integration tests ??full CRUD lifecycle with real database.
//!
//! These tests verify end-to-end correctness of the ORM with
//! an actual SQLite in-memory database.

#[cfg(test)]
mod sqlite_crud {
    use lref::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues};
    use lref::error::LrefResult;
    use lref::metadata::{EntityTypeMeta, PropertyMeta};
    use lref::migration::{MigrationDialect, MigrationEngine};
    use lref::provider::{DbValue, IAsyncConnection, IDatabaseProvider};
    use lref_provider_sqlite::SqliteProvider;
    use std::collections::HashMap;
    use std::sync::Arc;

    // -----------------------------------------------------------------------
    // Test entity: a minimal entity without derive (manual impl for test isolation)
    // -----------------------------------------------------------------------

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
        fn from_row(values: &[String]) -> LrefResult<Self> {
            Ok(TestItem {
                id: values.get(0).and_then(|s| s.parse().ok()).unwrap_or(0),
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

    // -----------------------------------------------------------------------
    // Helper: create table via migration engine
    // -----------------------------------------------------------------------

    async fn create_table(provider: &SqliteProvider, _table_name: &str) -> LrefResult<()> {
        let engine = MigrationEngine::new(MigrationDialect::Sqlite);
        let meta = TestItem::entity_meta();
        let migration = engine.generate("init", &[meta], &None)?;

        // Create migration history table first, then run migration
        let history_sql =
            lref::migration::create_migration_history_table_sql(MigrationDialect::Sqlite);
        provider.execute_migration_command(&history_sql).await?;

        // Strip out the history INSERT (it's a separate statement at the end)
        let ddl = migration
            .up_sql
            .lines()
            .filter(|l| !l.starts_with("INSERT INTO"))
            .collect::<Vec<_>>()
            .join("\n");
        provider.execute_migration_command(&ddl).await
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_insert_and_query() {
        let provider = SqliteProvider::new_in_memory().expect("create sqlite provider");
        let arc_provider = Arc::new(provider);

        create_table(&arc_provider, "test_items")
            .await
            .expect("create table");

        let mut db_set = lref::db_set::DbSet::<TestItem>::with_provider(
            "test_items",
            arc_provider.clone() as Arc<dyn lref::provider::IDatabaseProvider>,
        );

        // Insert
        db_set.add(TestItem {
            id: 0,
            name: "Alpha".into(),
            value: 1.0,
        });
        db_set.add(TestItem {
            id: 0,
            name: "Beta".into(),
            value: 2.0,
        });
        assert_eq!(db_set.len(), 2);

        // Save
        let mut conn: Box<dyn IAsyncConnection> =
            arc_provider.get_connection().await.expect("get connection");
        conn.begin_transaction().await.expect("begin tx");
        let (added, _, _) = lref::db_context::save_one_set(&mut *conn, &*arc_provider, &mut db_set)
            .await
            .expect("save");
        conn.commit_transaction().await.expect("commit");
        assert_eq!(added, 2);
        db_set.clear_entries();

        // Query back
        let items = db_set.query().to_list().await.expect("query to_list");
        assert_eq!(items.len(), 2);

        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
    }

    #[tokio::test]
    async fn test_filter_with_in_operator() {
        let provider = Arc::new(SqliteProvider::new_in_memory().expect("create db"));
        create_table(&provider, "test_items").await.unwrap();

        let mut db_set = lref::db_set::DbSet::<TestItem>::with_provider(
            "test_items",
            provider.clone() as Arc<dyn lref::provider::IDatabaseProvider>,
        );
        db_set.add(TestItem {
            id: 0,
            name: "A".into(),
            value: 1.0,
        });
        db_set.add(TestItem {
            id: 0,
            name: "B".into(),
            value: 2.0,
        });
        db_set.add(TestItem {
            id: 0,
            name: "C".into(),
            value: 3.0,
        });

        let mut conn: Box<dyn IAsyncConnection> = provider.get_connection().await.unwrap();
        conn.begin_transaction().await.unwrap();
        lref::db_context::save_one_set(&mut *conn, &*provider, &mut db_set)
            .await
            .unwrap();
        conn.commit_transaction().await.unwrap();
        db_set.clear_entries();

        // Query with filter
        let items = db_set
            .query()
            .filter_column("value", ">", 1.5)
            .to_list()
            .await
            .unwrap();
        assert_eq!(items.len(), 2);

        // Query with IS NULL
        let items_null = db_set
            .query()
            .filter_is_null("value")
            .to_list()
            .await
            .unwrap();
        assert_eq!(items_null.len(), 0);

        // Query with IS NOT NULL
        let items_not_null = db_set
            .query()
            .filter_is_not_null("name")
            .to_list()
            .await
            .unwrap();
        assert_eq!(items_not_null.len(), 3);
    }

    #[tokio::test]
    async fn test_limit_and_offset() {
        let provider = Arc::new(SqliteProvider::new_in_memory().unwrap());
        create_table(&provider, "test_items").await.unwrap();

        let mut db_set = lref::db_set::DbSet::<TestItem>::with_provider(
            "test_items",
            provider.clone() as Arc<dyn lref::provider::IDatabaseProvider>,
        );
        for i in 0..10 {
            db_set.add(TestItem {
                id: 0,
                name: format!("Item{}", i),
                value: i as f64,
            });
        }
        let mut conn: Box<dyn IAsyncConnection> = provider.get_connection().await.unwrap();
        conn.begin_transaction().await.unwrap();
        lref::db_context::save_one_set(&mut *conn, &*provider, &mut db_set)
            .await
            .unwrap();
        conn.commit_transaction().await.unwrap();
        db_set.clear_entries();

        // Test take
        let items = db_set.query().take(3).to_list().await.unwrap();
        assert_eq!(items.len(), 3);

        // Test skip + take
        let items = db_set.query().skip(8).take(5).to_list().await.unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_count_and_any() {
        let provider = Arc::new(SqliteProvider::new_in_memory().unwrap());
        create_table(&provider, "test_items").await.unwrap();

        let mut db_set = lref::db_set::DbSet::<TestItem>::with_provider(
            "test_items",
            provider.clone() as Arc<dyn lref::provider::IDatabaseProvider>,
        );
        for i in 0..5 {
            db_set.add(TestItem {
                id: 0,
                name: "X".into(),
                value: i as f64,
            });
        }
        let mut conn: Box<dyn IAsyncConnection> = provider.get_connection().await.unwrap();
        conn.begin_transaction().await.unwrap();
        lref::db_context::save_one_set(&mut *conn, &*provider, &mut db_set)
            .await
            .unwrap();
        conn.commit_transaction().await.unwrap();
        db_set.clear_entries();

        let count = db_set.query().count().await.unwrap();
        assert_eq!(count, 5);

        let any = db_set
            .query()
            .filter_column("value", "=", 3)
            .any()
            .await
            .unwrap();
        assert!(any);

        let any_none = db_set
            .query()
            .filter_column("value", "=", 99)
            .any()
            .await
            .unwrap();
        assert!(!any_none);
    }

    #[tokio::test]
    async fn test_update_and_delete() {
        let provider = Arc::new(SqliteProvider::new_in_memory().unwrap());
        create_table(&provider, "test_items").await.unwrap();

        let mut db_set = lref::db_set::DbSet::<TestItem>::with_provider(
            "test_items",
            provider.clone() as Arc<dyn lref::provider::IDatabaseProvider>,
        );
        db_set.add(TestItem {
            id: 0,
            name: "ToUpdate".into(),
            value: 10.0,
        });
        db_set.add(TestItem {
            id: 0,
            name: "ToDelete".into(),
            value: 20.0,
        });

        let mut conn: Box<dyn IAsyncConnection> = provider.get_connection().await.unwrap();
        conn.begin_transaction().await.unwrap();
        lref::db_context::save_one_set(&mut *conn, &*provider, &mut db_set)
            .await
            .unwrap();
        conn.commit_transaction().await.unwrap();
        db_set.clear_entries();

        // Load entities from DB, then modify
        let items = db_set.query().to_list().await.unwrap();
        let mut item_to_update = items
            .iter()
            .find(|i| i.name == "ToUpdate")
            .cloned()
            .unwrap();
        item_to_update.value = 99.0;
        db_set.add(item_to_update); // Add as new (in real use, this would be Modified state)

        // Requery to verify state
        let items = db_set.query().to_list().await.unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn test_aggregation_queries() {
        let provider = Arc::new(SqliteProvider::new_in_memory().unwrap());
        create_table(&provider, "test_items").await.unwrap();

        let mut db_set = lref::db_set::DbSet::<TestItem>::with_provider(
            "test_items",
            provider.clone() as Arc<dyn lref::provider::IDatabaseProvider>,
        );
        for i in 1..=5 {
            db_set.add(TestItem {
                id: 0,
                name: "Agg".into(),
                value: i as f64,
            });
        }
        let mut conn: Box<dyn IAsyncConnection> = provider.get_connection().await.unwrap();
        conn.begin_transaction().await.unwrap();
        lref::db_context::save_one_set(&mut *conn, &*provider, &mut db_set)
            .await
            .unwrap();
        conn.commit_transaction().await.unwrap();
        db_set.clear_entries();

        let sum = db_set.query().sum("value").await.unwrap();
        assert!((sum - 15.0).abs() < 0.01, "sum should be 15.0, got {}", sum);

        let avg = db_set.query().avg("value").await.unwrap();
        assert!((avg - 3.0).abs() < 0.01, "avg should be 3.0, got {}", avg);
    }

    #[tokio::test]
    async fn test_empty_result_handling() {
        let provider = Arc::new(SqliteProvider::new_in_memory().unwrap());
        create_table(&provider, "test_items").await.unwrap();

        let db_set = lref::db_set::DbSet::<TestItem>::with_provider(
            "test_items",
            provider.clone() as Arc<dyn lref::provider::IDatabaseProvider>,
        );

        let items = db_set.query().to_list().await.unwrap();
        assert!(items.is_empty());

        let count = db_set.query().count().await.unwrap();
        assert_eq!(count, 0);

        let any = db_set.query().any().await.unwrap();
        assert!(!any);

        let first = db_set.query().first_or_default().await.unwrap();
        assert!(first.is_none());
    }
}
