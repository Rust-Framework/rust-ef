#[cfg(test)]
mod advanced_tests {
    use rust_ef::entity::EntityState;
    use rust_ef::metadata::{EntityTypeMeta, NavigationKind, NavigationMeta, PropertyMeta};
    use rust_ef::migration::{MigrationDialect, MigrationEngine};

    fn make_pk_col(name: &'static str, fk: bool) -> PropertyMeta {
        PropertyMeta {
            field_name: std::borrow::Cow::Borrowed(name),
            column_name: std::borrow::Cow::Borrowed(name),
            type_id: std::any::TypeId::of::<i32>(),
            type_name: std::borrow::Cow::Borrowed("i32"),
            is_primary_key: true,
            is_auto_increment: false,
            is_sequence: false,
            sequence_name: None,
            is_required: true,
            is_foreign_key: fk,
            is_concurrency_token: false,
            max_length: None,
            is_unique: false,
            has_index: false,
            is_not_mapped: false,
        }
    }

    #[test]
    fn test_composite_primary_key_migration() {
        let engine = MigrationEngine::new(MigrationDialect::Postgres);
        let user_role_meta = EntityTypeMeta {
            type_id: std::any::TypeId::of::<i32>(),
            type_name: std::borrow::Cow::Borrowed("UserRole"),
            table_name: std::borrow::Cow::Borrowed("user_roles"),
            properties: vec![make_pk_col("user_id", true), make_pk_col("role_id", true)],
            navigations: vec![],
            primary_keys: vec![
                std::borrow::Cow::Borrowed("user_id"),
                std::borrow::Cow::Borrowed("role_id"),
            ],
            ..EntityTypeMeta::default()
        };
        let migration = engine
            .generate("InitialCreate", &[user_role_meta], &None)
            .expect("generate should succeed");
        assert!(migration.up_sql.contains("PRIMARY KEY"));
        assert!(migration.up_sql.contains("user_id"));
        assert!(migration.up_sql.contains("role_id"));
    }

    #[test]
    fn test_single_primary_key_migration() {
        let engine = MigrationEngine::new(MigrationDialect::Postgres);
        let blog_meta = EntityTypeMeta {
            type_id: std::any::TypeId::of::<i32>(),
            type_name: std::borrow::Cow::Borrowed("Blog"),
            table_name: std::borrow::Cow::Borrowed("blogs"),
            properties: vec![PropertyMeta {
                field_name: std::borrow::Cow::Borrowed("blog_id"),
                column_name: std::borrow::Cow::Borrowed("blog_id"),
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
            }],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("blog_id")],
            ..EntityTypeMeta::default()
        };
        let migration = engine.generate("Initial", &[blog_meta], &None).unwrap();
        assert!(migration.up_sql.contains("CREATE TABLE"));
        assert!(migration.up_sql.contains("blogs"));
        assert!(migration.up_sql.contains("SERIAL"));
    }

    #[test]
    fn test_migration_down_sql_reverses_create() {
        let engine = MigrationEngine::new(MigrationDialect::Sqlite);
        let meta = EntityTypeMeta {
            type_id: std::any::TypeId::of::<i32>(),
            type_name: std::borrow::Cow::Borrowed("Test"),
            table_name: std::borrow::Cow::Borrowed("tests"),
            properties: vec![PropertyMeta {
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
            }],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("id")],
            ..EntityTypeMeta::default()
        };
        let migration = engine.generate("Test", &[meta], &None).unwrap();
        assert!(migration.up_sql.contains("CREATE TABLE"));
        assert!(migration.down_sql.contains("DROP TABLE"));
    }

    #[test]
    fn test_model_snapshot_creation() {
        let engine = MigrationEngine::new(MigrationDialect::Postgres);
        let metas = vec![EntityTypeMeta {
            type_id: std::any::TypeId::of::<i32>(),
            type_name: std::borrow::Cow::Borrowed("Post"),
            table_name: std::borrow::Cow::Borrowed("posts"),
            properties: vec![
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("post_id"),
                    column_name: std::borrow::Cow::Borrowed("post_id"),
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
                    field_name: std::borrow::Cow::Borrowed("title"),
                    column_name: std::borrow::Cow::Borrowed("title"),
                    type_id: std::any::TypeId::of::<String>(),
                    type_name: std::borrow::Cow::Borrowed("String"),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_sequence: false,
                    sequence_name: None,
                    is_required: true,
                    is_foreign_key: false,
                    is_concurrency_token: false,
                    max_length: Some(200),
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                },
            ],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("post_id")],
            ..EntityTypeMeta::default()
        }];
        let snapshot = engine.create_snapshot("v1", &metas);
        assert_eq!(snapshot.entity_types.len(), 1);
        assert_eq!(snapshot.entity_types[0].columns.len(), 2);
        assert_eq!(snapshot.entity_types[0].table_name, "posts");
    }

    #[test]
    fn test_foreign_key_migration_from_navigation() {
        let engine = MigrationEngine::new(MigrationDialect::Sqlite);
        let blog_meta = EntityTypeMeta {
            type_id: std::any::TypeId::of::<i32>(),
            type_name: std::borrow::Cow::Borrowed("Blog"),
            table_name: std::borrow::Cow::Borrowed("blogs"),
            properties: vec![PropertyMeta {
                field_name: std::borrow::Cow::Borrowed("blog_id"),
                column_name: std::borrow::Cow::Borrowed("blog_id"),
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
            }],
            navigations: vec![],
            primary_keys: vec![std::borrow::Cow::Borrowed("blog_id")],
            ..EntityTypeMeta::default()
        };
        let post_meta = EntityTypeMeta {
            type_id: std::any::TypeId::of::<i64>(),
            type_name: std::borrow::Cow::Borrowed("Post"),
            table_name: std::borrow::Cow::Borrowed("posts"),
            properties: vec![
                PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed("post_id"),
                    column_name: std::borrow::Cow::Borrowed("post_id"),
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
                    field_name: std::borrow::Cow::Borrowed("blog_id"),
                    column_name: std::borrow::Cow::Borrowed("blog_id"),
                    type_id: std::any::TypeId::of::<i32>(),
                    type_name: std::borrow::Cow::Borrowed("i32"),
                    is_primary_key: false,
                    is_auto_increment: false,
                    is_sequence: false,
                    sequence_name: None,
                    is_required: true,
                    is_foreign_key: true,
                    is_concurrency_token: false,
                    max_length: None,
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                },
            ],
            navigations: vec![NavigationMeta {
                field_name: std::borrow::Cow::Borrowed("blog"),
                kind: NavigationKind::BelongsTo,
                related_type_id: std::any::TypeId::of::<i32>(),
                related_type_name: std::borrow::Cow::Borrowed("Blog"),
                foreign_key_field: Some(std::borrow::Cow::Borrowed("blog_id")),
                inverse_navigation: None,
                through_type_id: None,
                through_table: None,
                through_parent_fk: None,
                through_related_fk: None,
                through_parent_fk_index: 0,
                through_related_fk_index: 0,
                related_table: Some(std::borrow::Cow::Borrowed("blogs")),
                fk_column: Some(std::borrow::Cow::Borrowed("blog_id")),
                referenced_key_column: Some(std::borrow::Cow::Borrowed("blog_id")),
                fk_row_index: 1,
                pk_row_index: 0,
                related_entity_meta: None,
            }],
            primary_keys: vec![std::borrow::Cow::Borrowed("post_id")],
            ..EntityTypeMeta::default()
        };

        let migration = engine
            .generate("Initial", &[blog_meta, post_meta], &None)
            .unwrap();
        assert!(migration.up_sql.contains("CREATE TABLE"));
        assert!(migration.up_sql.contains("FOREIGN KEY"));
        assert!(migration.up_sql.contains("fk_posts_blog_id_blogs"));
    }

    #[test]
    fn test_change_tracker_states() {
        let mut tracker = rust_ef::tracking::ChangeTracker::new();
        let type_id = std::any::TypeId::of::<i32>();
        tracker.track_entity(type_id, "UserRole", EntityState::Added);
        assert!(tracker.has_changes());
        assert_eq!(tracker.count_by_state(EntityState::Added), 1);
        tracker.accept_all_changes();
        assert!(!tracker.has_changes());
    }
}
