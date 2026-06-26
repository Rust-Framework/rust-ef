#[cfg(test)]
mod tests {
    use rust_ef::entity::EntityState;
    use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
    use rust_ef::migration::{MigrationDialect, SnapshotColumn};
    use rust_ef::provider::DbValue;
    use rust_ef::query::{FilterCondition, OrderBy, OrderDirection, QueryState};

    #[test]
    fn test_entity_state_enum() {
        assert_ne!(EntityState::Added, EntityState::Unchanged);
        assert_ne!(EntityState::Modified, EntityState::Deleted);
    }

    #[test]
    fn test_query_state_to_sql_basic() {
        let state = QueryState::new("blogs");
        let sql = state.to_sql();
        assert_eq!(sql, "SELECT * FROM blogs");
    }

    #[test]
    fn test_query_state_to_sql_with_filter() {
        let mut state = QueryState::new("blogs");
        state.filters.push(FilterCondition::new("rating", ">", 1));
        state.parameters.push(DbValue::I32(3));
        let sql = state.to_sql();
        assert!(sql.contains("WHERE"));
        assert!(sql.contains("rating"));
        assert!(sql.contains("?"));
    }

    #[test]
    fn test_query_state_to_sql_with_order() {
        let mut state = QueryState::new("posts");
        state
            .orderings
            .push(OrderBy::new("title", OrderDirection::Ascending));
        let sql = state.to_sql();
        assert!(sql.contains("ORDER BY"));
    }

    #[test]
    fn test_query_state_to_sql_pagination() {
        let mut state = QueryState::new("posts");
        state.limit = Some(10);
        state.offset = Some(20);
        let sql = state.to_sql();
        assert!(sql.contains("LIMIT 10"));
        assert!(sql.contains("OFFSET 20"));
    }

    #[test]
    fn test_query_state_to_sql_count() {
        let mut state = QueryState::new("posts");
        state.is_count = true;
        let sql = state.to_sql();
        assert!(sql.contains("SELECT COUNT(*)"));
    }

    #[test]
    fn test_save_changes_result() {
        let result = rust_ef::db_context::SaveChangesResult {
            added: 3,
            updated: 2,
            deleted: 1,
        };
        assert_eq!(result.total(), 6);
        let display = format!("{}", result);
        assert!(display.contains("3 added"));
    }

    #[test]
    fn test_metadata_entity_type() {
        let mut meta = EntityTypeMeta {
            type_id: std::any::TypeId::of::<i32>(),
            type_name: std::borrow::Cow::Borrowed("TestEntity"),
            table_name: std::borrow::Cow::Borrowed("test_entities"),
            properties: vec![],
            navigations: vec![],
            primary_keys: vec![],
            ..EntityTypeMeta::default()
        };

        meta.properties.push(PropertyMeta {
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
        });

        assert!(meta.primary_key_property().is_some());
        assert_eq!(meta.mapped_scalar_properties().count(), 1);
    }

    #[test]
    fn test_migration_dialect_quote() {
        assert_eq!(MigrationDialect::Postgres.quote("test"), "\"test\"");
        assert_eq!(MigrationDialect::MySql.quote("test"), "`test`");
        assert_eq!(MigrationDialect::Sqlite.quote("test"), "\"test\"");
    }

    #[test]
    fn test_migration_foreign_key_naming() {
        let name =
            rust_ef::migration::MigrationEngine::foreign_key_name("posts", "blog_id", "blogs");
        assert_eq!(name, "fk_posts_blog_id_blogs");
    }

    #[test]
    fn test_migration_binary_type_mapping() {
        let col = SnapshotColumn {
            field_name: "data".into(),
            column_name: "data".into(),
            type_name: "Vec<u8>".into(),
            is_primary_key: false,
            is_required: false,
            is_foreign_key: false,
            max_length: None,
            is_auto_increment: false,
            fk_referenced_table: None,
            fk_referenced_column: None,
            has_index: false,
            is_unique: false,
        };
        assert_eq!(MigrationDialect::Postgres.map_column_type(&col), "BYTEA");
        assert_eq!(MigrationDialect::MySql.map_column_type(&col), "BLOB");
        assert_eq!(MigrationDialect::Sqlite.map_column_type(&col), "BLOB");
    }

    #[test]
    fn test_migration_alter_column_mysql() {
        let engine = rust_ef::migration::MigrationEngine::new(MigrationDialect::MySql);
        let col = SnapshotColumn {
            field_name: "title".into(),
            column_name: "title".into(),
            type_name: "String".into(),
            is_primary_key: false,
            is_required: true,
            is_foreign_key: false,
            max_length: Some(200),
            is_auto_increment: false,
            fk_referenced_table: None,
            fk_referenced_column: None,
            has_index: false,
            is_unique: false,
        };
        let sql = engine.generate_alter_column_sql("posts", "title", &col);
        assert!(sql.contains("MODIFY COLUMN"));
        assert!(sql.contains("VARCHAR(200)"));
    }

    #[test]
    fn test_migration_dialect_type_mapping() {
        let col = SnapshotColumn {
            field_name: "id".into(),
            column_name: "id".into(),
            type_name: "i32".into(),
            is_primary_key: true,
            is_required: true,
            is_foreign_key: false,
            max_length: None,
            is_auto_increment: true,
            fk_referenced_table: None,
            fk_referenced_column: None,
            has_index: false,
            is_unique: false,
        };
        assert_eq!(MigrationDialect::Postgres.map_column_type(&col), "SERIAL");
        assert_eq!(
            MigrationDialect::MySql.map_column_type(&col),
            "INT AUTO_INCREMENT"
        );
        assert_eq!(MigrationDialect::Sqlite.map_column_type(&col), "INTEGER");
    }

    #[test]
    fn test_db_value_display() {
        assert_eq!(format!("{}", DbValue::I32(42)), "42");
        assert_eq!(format!("{}", DbValue::Null), "NULL");
    }

    #[test]
    fn test_db_value_from_traits() {
        let v: DbValue = 42i32.into();
        assert!(matches!(v, DbValue::I32(42)));

        let v: DbValue = "hello".into();
        assert!(matches!(v, DbValue::String(s) if s == "hello"));

        let v: DbValue = true.into();
        assert!(matches!(v, DbValue::Bool(true)));

        let v: DbValue = <Option<i32>>::None.into();
        assert!(matches!(v, DbValue::Null));
    }

    #[test]
    fn test_change_tracker() {
        let mut tracker = rust_ef::tracking::ChangeTracker::new();
        assert!(!tracker.has_changes());

        let type_id = std::any::TypeId::of::<i32>();
        tracker.track_entity(type_id, "i32", EntityState::Added);
        assert!(tracker.has_changes());

        tracker.accept_all_changes();
        assert!(!tracker.has_changes());
    }
}
