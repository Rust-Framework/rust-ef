//! Integration tests for optimistic concurrency and transaction rollback.

#[cfg(test)]
mod production_tests {
    use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
    use rust_ef::entity::{
        IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter,
    };
    use rust_ef::error::EFError;
    use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
    use rust_ef::migration::{MigrationDialect, MigrationEngine};
    use rust_ef::provider::{DbValue, IDatabaseProvider};
    use rust_ef_sqlite::SqliteProvider;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Debug, Clone, Default, PartialEq)]
    struct VersionedItem {
        id: i32,
        name: String,
        row_version: i32,
    }

    impl IEntityType for VersionedItem {
        fn entity_meta() -> EntityTypeMeta {
            EntityTypeMeta {
                type_id: std::any::TypeId::of::<Self>(),
                type_name: std::borrow::Cow::Borrowed("VersionedItem"),
                table_name: std::borrow::Cow::Borrowed("versioned_items"),
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
                        max_length: None,
                        is_unique: false,
                        has_index: false,
                        is_not_mapped: false,
                    },
                    PropertyMeta {
                        field_name: std::borrow::Cow::Borrowed("row_version"),
                        column_name: std::borrow::Cow::Borrowed("row_version"),
                        type_id: std::any::TypeId::of::<i32>(),
                        type_name: std::borrow::Cow::Borrowed("i32"),
                        is_primary_key: false,
                        is_auto_increment: false,
                        is_required: true,
                        is_foreign_key: false,
                        is_concurrency_token: true,
                        max_length: None,
                        is_unique: false,
                        has_index: false,
                        is_not_mapped: false,
                    },
                ],
                navigations: Vec::new(),
                primary_keys: vec![std::borrow::Cow::Borrowed("id")],
                ..EntityTypeMeta::default()
            }
        }
    }

    impl INavigationSetter for VersionedItem {}

    impl rust_ef::entity::ILazyInit for VersionedItem {
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

    impl IFromRow for VersionedItem {
        fn from_row(values: &[String]) -> rust_ef::error::EFResult<Self> {
            Ok(Self {
                id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
                name: values.get(1).cloned().unwrap_or_default(),
                row_version: values.get(2).and_then(|v| v.parse().ok()).unwrap_or(0),
            })
        }
    }

    impl IGetKeyValues for VersionedItem {
        fn key_values(&self) -> HashMap<String, DbValue> {
            let mut m = HashMap::new();
            m.insert("id".into(), DbValue::I32(self.id));
            m
        }
    }

    impl IEntitySnapshot for VersionedItem {
        fn snapshot(&self) -> HashMap<String, DbValue> {
            let mut m = HashMap::new();
            m.insert("id".into(), DbValue::I32(self.id));
            m.insert("name".into(), DbValue::String(self.name.clone()));
            m.insert("row_version".into(), DbValue::I32(self.row_version));
            m
        }
    }

    #[allow(clippy::type_complexity)]
    async fn setup_ctx() -> (DbContext, Arc<SqliteProvider>) {
        let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
        let meta = VersionedItem::entity_meta();
        let engine = MigrationEngine::new(MigrationDialect::Sqlite);
        engine.ensure_created(&*provider, &[meta]).await.unwrap();

        let factory: Arc<
            dyn Fn(&str) -> rust_ef::error::EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync,
        > = {
            let p = provider.clone();
            Arc::new(move |_| Ok(p.clone() as Arc<dyn IDatabaseProvider>))
        };
        let mut builder = DbContextOptionsBuilder::new();
        builder.connection_string(":memory:");
        builder.set_provider_factory("sqlite", ":memory:", factory);
        let options = builder.build();
        let ctx = DbContext::from_options(&options).unwrap();
        (ctx, provider)
    }

    #[tokio::test]
    async fn test_concurrency_conflict_on_stale_token() {
        let (mut ctx, _provider) = setup_ctx().await;
        ctx.set::<VersionedItem>().add(VersionedItem {
            id: 0,
            name: "alpha".into(),
            row_version: 1,
        });
        ctx.save_changes().await.unwrap();

        let mut loaded = ctx.set::<VersionedItem>().query().to_list().await.unwrap();
        assert_eq!(loaded.len(), 1);
        let entity = loaded.remove(0);
        ctx.set::<VersionedItem>().clear_entries();
        ctx.set::<VersionedItem>().attach(entity);
        ctx.set::<VersionedItem>()
            .tracked_entries_mut()
            .next()
            .unwrap()
            .name = "beta".into();

        // Simulate another writer bumping row_version in the database
        let mut conn = _provider.get_connection().await.unwrap();
        conn.execute(
            "UPDATE versioned_items SET row_version = 99 WHERE id = 1",
            &[],
        )
        .await
        .unwrap();

        ctx.set::<VersionedItem>().detect_changes();
        let result = ctx.save_changes().await;
        assert!(matches!(result, Err(EFError::ConcurrencyConflict(_))));
    }

    #[tokio::test]
    async fn test_save_changes_rollback_on_failure() {
        let (mut ctx, provider) = setup_ctx().await;
        ctx.set::<VersionedItem>().add(VersionedItem {
            id: 0,
            name: "will_commit".into(),
            row_version: 1,
        });
        ctx.save_changes().await.unwrap();

        ctx.set::<VersionedItem>().add(VersionedItem {
            id: 0,
            name: "second".into(),
            row_version: 1,
        });
        ctx.save_changes().await.unwrap();

        let stale = ctx
            .set::<VersionedItem>()
            .query()
            .to_list()
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.name == "will_commit")
            .unwrap();
        ctx.set::<VersionedItem>().clear_entries();
        ctx.set::<VersionedItem>().attach(stale);
        ctx.set::<VersionedItem>()
            .tracked_entries_mut()
            .next()
            .unwrap()
            .name = "updated".into();
        ctx.set::<VersionedItem>().detect_changes();

        let mut conn = provider.get_connection().await.unwrap();
        conn.execute(
            "UPDATE versioned_items SET row_version = 50 WHERE name = 'will_commit'",
            &[],
        )
        .await
        .unwrap();

        let err = ctx.save_changes().await;
        assert!(matches!(err, Err(EFError::ConcurrencyConflict(_))));

        let row = ctx
            .set::<VersionedItem>()
            .query()
            .to_list()
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.name == "will_commit")
            .unwrap();
        assert_eq!(row.name, "will_commit");
    }

    #[tokio::test]
    async fn test_migration_apply_and_revert() {
        let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
        let engine = MigrationEngine::new(MigrationDialect::Sqlite);
        let meta = VersionedItem::entity_meta();
        let migration = engine.generate("Initial", &[meta], &None).unwrap();

        engine.apply(&*provider, &migration).await.unwrap();
        assert!(engine.is_applied(&*provider, "Initial").await.unwrap());

        let applied = engine.get_applied_migrations(&*provider).await.unwrap();
        assert_eq!(applied.len(), 1);

        engine.revert(&*provider, &migration).await.unwrap();
        assert!(!engine.is_applied(&*provider, "Initial").await.unwrap());
    }

    #[tokio::test]
    #[allow(clippy::type_complexity)]
    async fn test_composite_primary_key_crud() {
        #[derive(Debug, Clone, Default, PartialEq)]
        struct UserRole {
            user_id: i32,
            role_id: i32,
            label: String,
        }

        impl INavigationSetter for UserRole {}

        impl rust_ef::entity::ILazyInit for UserRole {
            fn attach_lazy_contexts(
                &mut self,
                _provider: std::sync::Arc<dyn rust_ef::provider::IDatabaseProvider>,
                _filter_map: Option<
                    std::sync::Arc<
                        std::collections::HashMap<String, rust_ef::query::CompiledFilter>,
                    >,
                >,
                _depth: usize,
            ) {
                // No navigation fields — lazy loading is a no-op for this test entity.
            }
        }

        impl IEntityType for UserRole {
            fn entity_meta() -> EntityTypeMeta {
                EntityTypeMeta {
                    type_id: std::any::TypeId::of::<Self>(),
                    type_name: std::borrow::Cow::Borrowed("UserRole"),
                    table_name: std::borrow::Cow::Borrowed("user_roles"),
                    properties: vec![
                        PropertyMeta {
                            field_name: std::borrow::Cow::Borrowed("user_id"),
                            column_name: std::borrow::Cow::Borrowed("user_id"),
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
                            field_name: std::borrow::Cow::Borrowed("role_id"),
                            column_name: std::borrow::Cow::Borrowed("role_id"),
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
                            field_name: std::borrow::Cow::Borrowed("label"),
                            column_name: std::borrow::Cow::Borrowed("label"),
                            type_id: std::any::TypeId::of::<String>(),
                            type_name: std::borrow::Cow::Borrowed("String"),
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
                    navigations: Vec::new(),
                    primary_keys: vec![
                        std::borrow::Cow::Borrowed("user_id"),
                        std::borrow::Cow::Borrowed("role_id"),
                    ],
                    ..EntityTypeMeta::default()
                }
            }
        }

        impl IFromRow for UserRole {
            fn from_row(values: &[String]) -> rust_ef::error::EFResult<Self> {
                Ok(Self {
                    user_id: values.first().and_then(|v| v.parse().ok()).unwrap_or(0),
                    role_id: values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0),
                    label: values.get(2).cloned().unwrap_or_default(),
                })
            }
        }

        impl IGetKeyValues for UserRole {
            fn key_values(&self) -> HashMap<String, DbValue> {
                let mut m = HashMap::new();
                m.insert("user_id".into(), DbValue::I32(self.user_id));
                m.insert("role_id".into(), DbValue::I32(self.role_id));
                m
            }
        }

        impl IEntitySnapshot for UserRole {
            fn snapshot(&self) -> HashMap<String, DbValue> {
                let mut m = HashMap::new();
                m.insert("user_id".into(), DbValue::I32(self.user_id));
                m.insert("role_id".into(), DbValue::I32(self.role_id));
                m.insert("label".into(), DbValue::String(self.label.clone()));
                m
            }
        }

        let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
        let engine = MigrationEngine::new(MigrationDialect::Sqlite);
        engine
            .ensure_created(&*provider, &[UserRole::entity_meta()])
            .await
            .unwrap();

        let factory: Arc<
            dyn Fn(&str) -> rust_ef::error::EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync,
        > = {
            let p = provider.clone();
            Arc::new(move |_| Ok(p.clone() as Arc<dyn IDatabaseProvider>))
        };
        let mut builder = DbContextOptionsBuilder::new();
        builder.connection_string(":memory:");
        builder.set_provider_factory("sqlite", ":memory:", factory);
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<UserRole>().add(UserRole {
            user_id: 1,
            role_id: 2,
            label: "admin".into(),
        });
        ctx.save_changes().await.unwrap();

        let rows = ctx.set::<UserRole>().query().to_list().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "admin");

        ctx.set::<UserRole>().attach(rows[0].clone());
        ctx.set::<UserRole>().remove_at(0).unwrap();
        ctx.save_changes().await.unwrap();
        assert_eq!(ctx.set::<UserRole>().query().count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_migration_store_apply_pending() {
        use rust_ef::migration::MigrationStore;

        let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
        let engine = MigrationEngine::new(MigrationDialect::Sqlite);
        let meta = VersionedItem::entity_meta();
        let migration = engine.generate("Init", &[meta], &None).unwrap();

        let dir =
            std::env::temp_dir().join(format!("rust_ef_migration_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = MigrationStore::new(&dir);
        store.save(&migration).unwrap();

        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.len(), 1);

        let applied = engine.apply_pending(&*provider, &loaded).await.unwrap();
        assert_eq!(applied, 1);
        assert!(engine.is_applied(&*provider, "Init").await.unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_exists_by_id() {
        let (mut ctx, _provider) = setup_ctx().await;
        ctx.set::<VersionedItem>().add(VersionedItem {
            id: 0,
            name: "exists".into(),
            row_version: 1,
        });
        ctx.save_changes().await.unwrap();

        let mut keys = HashMap::new();
        keys.insert("id".into(), DbValue::I32(1));
        assert!(ctx
            .set::<VersionedItem>()
            .exists_by_id(keys.clone())
            .await
            .unwrap());

        keys.insert("id".into(), DbValue::I32(999));
        assert!(!ctx.set::<VersionedItem>().exists_by_id(keys).await.unwrap());
    }
}
