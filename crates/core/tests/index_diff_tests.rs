//! Integration tests for index diff in migration generation.
//!
//! Verifies that `MigrationEngine::generate` emits `CREATE INDEX` / `DROP INDEX`
//! statements when columns' `has_index` / `is_unique` flags change between
//! snapshots, and that index changes do NOT trigger spurious `ALTER COLUMN`.

use rust_ef::metadata::{EntityTypeMeta, PropertyMeta};
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::provider::IDatabaseProvider;
use std::borrow::Cow;

fn make_prop(name: &str, type_name: &str, has_index: bool, is_unique: bool) -> PropertyMeta {
    PropertyMeta {
        field_name: Cow::Owned(name.to_string()),
        column_name: Cow::Owned(name.to_string()),
        type_id: std::any::TypeId::of::<i32>(),
        type_name: Cow::Owned(type_name.to_string()),
        is_primary_key: name == "id",
        is_auto_increment: name == "id",
        is_sequence: false,
        sequence_name: None,
        is_required: true,
        is_foreign_key: false,
        is_concurrency_token: false,
        max_length: None,
        is_unique,
        has_index,
        is_not_mapped: false,
    }
}

fn make_meta(table: &str, props: Vec<PropertyMeta>) -> EntityTypeMeta {
    EntityTypeMeta {
        type_id: std::any::TypeId::of::<i32>(),
        type_name: Cow::Owned(table.to_string()),
        table_name: Cow::Owned(table.to_string()),
        properties: props,
        navigations: Vec::new(),
        primary_keys: vec![Cow::Owned("id".to_string())],
        ..EntityTypeMeta::default()
    }
}

// ---------------------------------------------------------------------------
// New table with indexes
// ---------------------------------------------------------------------------

#[test]
fn test_new_table_with_indexed_column() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", true, false),
        ],
    );
    let migration = engine.generate("AddPosts", &[meta], &None).unwrap();

    assert!(
        migration.up_sql.contains("CREATE INDEX") || migration.up_sql.contains("CREATE  INDEX"),
        "up_sql should contain CREATE INDEX: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("\"ix_posts_title\""),
        "up_sql should contain index name ix_posts_title: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("\"posts\" (\"title\")"),
        "up_sql should contain table/column: {}",
        migration.up_sql
    );
}

#[test]
fn test_new_table_with_unique_column() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let meta = make_meta(
        "users",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("email", "String", false, true),
        ],
    );
    let migration = engine.generate("AddUsers", &[meta], &None).unwrap();

    assert!(
        migration.up_sql.contains("CREATE UNIQUE INDEX"),
        "up_sql should contain CREATE UNIQUE INDEX: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("\"ix_users_email\""),
        "up_sql should contain index name ix_users_email: {}",
        migration.up_sql
    );
}

// ---------------------------------------------------------------------------
// Add index to existing column
// ---------------------------------------------------------------------------

#[test]
fn test_add_index_to_existing_column() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Old: column without index
    let old_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", false, false),
        ],
    );
    let old_snapshot = engine.create_snapshot("old", &[old_meta]);

    // New: same column with index
    let new_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", true, false),
        ],
    );
    let migration = engine
        .generate("AddIndex", &[new_meta], &Some(old_snapshot))
        .unwrap();

    assert!(
        migration.up_sql.contains("CREATE INDEX"),
        "up_sql should contain CREATE INDEX: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("\"ix_posts_title\""),
        "up_sql should contain index name: {}",
        migration.up_sql
    );
    // Should NOT contain ALTER COLUMN for the title (only index changed)
    assert!(
        !migration.up_sql.contains("ALTER TABLE \"posts\""),
        "up_sql should NOT contain ALTER TABLE for index-only change: {}",
        migration.up_sql
    );
    // Down SQL should DROP INDEX
    assert!(
        migration.down_sql.contains("DROP INDEX"),
        "down_sql should contain DROP INDEX: {}",
        migration.down_sql
    );
}

// ---------------------------------------------------------------------------
// Remove index from column
// ---------------------------------------------------------------------------

#[test]
fn test_remove_index_from_column() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Old: column with index
    let old_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", true, false),
        ],
    );
    let old_snapshot = engine.create_snapshot("old", &[old_meta]);

    // New: column without index
    let new_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", false, false),
        ],
    );
    let migration = engine
        .generate("RemoveIndex", &[new_meta], &Some(old_snapshot))
        .unwrap();

    assert!(
        migration.up_sql.contains("DROP INDEX"),
        "up_sql should contain DROP INDEX: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("\"ix_posts_title\""),
        "up_sql should contain index name: {}",
        migration.up_sql
    );
    // Down SQL should CREATE INDEX
    assert!(
        migration.down_sql.contains("CREATE INDEX"),
        "down_sql should contain CREATE INDEX: {}",
        migration.down_sql
    );
    // Should NOT contain ALTER TABLE
    assert!(
        !migration.up_sql.contains("ALTER TABLE \"posts\""),
        "up_sql should NOT contain ALTER TABLE: {}",
        migration.up_sql
    );
}

// ---------------------------------------------------------------------------
// Change non-unique to unique
// ---------------------------------------------------------------------------

#[test]
fn test_change_non_unique_to_unique() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Old: non-unique index
    let old_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("slug", "String", true, false),
        ],
    );
    let old_snapshot = engine.create_snapshot("old", &[old_meta]);

    // New: unique index
    let new_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("slug", "String", false, true),
        ],
    );
    let migration = engine
        .generate("MakeUnique", &[new_meta], &Some(old_snapshot))
        .unwrap();

    // Should DROP old non-unique index then CREATE UNIQUE INDEX
    assert!(
        migration.up_sql.contains("DROP INDEX"),
        "up_sql should contain DROP INDEX: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("CREATE UNIQUE INDEX"),
        "up_sql should contain CREATE UNIQUE INDEX: {}",
        migration.up_sql
    );
}

// ---------------------------------------------------------------------------
// Add column with index
// ---------------------------------------------------------------------------

#[test]
fn test_add_column_with_index() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let old_meta = make_meta("posts", vec![make_prop("id", "i32", false, false)]);
    let old_snapshot = engine.create_snapshot("old", &[old_meta]);

    let new_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("category", "String", true, false),
        ],
    );
    let migration = engine
        .generate("AddCategory", &[new_meta], &Some(old_snapshot))
        .unwrap();

    // Should contain both ADD COLUMN and CREATE INDEX
    assert!(
        migration.up_sql.contains("ADD COLUMN"),
        "up_sql should contain ADD COLUMN: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("CREATE INDEX"),
        "up_sql should contain CREATE INDEX: {}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("\"ix_posts_category\""),
        "up_sql should contain index name: {}",
        migration.up_sql
    );
}

// ---------------------------------------------------------------------------
// No index change → no CREATE/DROP INDEX
// ---------------------------------------------------------------------------

#[test]
fn test_no_index_change_no_index_sql() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let old_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", true, false),
        ],
    );
    let old_snapshot = engine.create_snapshot("old", &[old_meta]);

    // Same schema — no changes at all
    let new_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", true, false),
        ],
    );
    let migration = engine
        .generate("NoChange", &[new_meta], &Some(old_snapshot))
        .unwrap();

    assert!(
        !migration.up_sql.contains("CREATE INDEX"),
        "up_sql should NOT contain CREATE INDEX when no index changed: {}",
        migration.up_sql
    );
    assert!(
        !migration.up_sql.contains("DROP INDEX"),
        "up_sql should NOT contain DROP INDEX when no index changed: {}",
        migration.up_sql
    );
}

// ---------------------------------------------------------------------------
// MySQL dialect: DROP INDEX ... ON table
// ---------------------------------------------------------------------------

#[test]
fn test_mysql_drop_index_syntax() {
    let engine = MigrationEngine::new(MigrationDialect::MySql);

    let old_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", true, false),
        ],
    );
    let old_snapshot = engine.create_snapshot("old", &[old_meta]);

    let new_meta = make_meta(
        "posts",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("title", "String", false, false),
        ],
    );
    let migration = engine
        .generate("RemoveIndex", &[new_meta], &Some(old_snapshot))
        .unwrap();

    // MySQL: DROP INDEX `ix_posts_title` ON `posts`
    assert!(
        migration.up_sql.contains("DROP INDEX") && migration.up_sql.contains("ON `posts`"),
        "MySQL up_sql should contain 'DROP INDEX ... ON `posts`': {}",
        migration.up_sql
    );
}

// ---------------------------------------------------------------------------
// Index name helper
// ---------------------------------------------------------------------------

#[test]
fn test_index_name_helper() {
    assert_eq!(
        MigrationEngine::index_name("posts", "title"),
        "ix_posts_title"
    );
    assert_eq!(
        MigrationEngine::index_name("users", "email"),
        "ix_users_email"
    );
}

// ---------------------------------------------------------------------------
// End-to-end: apply migration with index to SQLite
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_apply_migration_creates_index_in_db() {
    use rust_ef_sqlite::SqliteProvider;
    use std::sync::Arc;

    let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let meta = make_meta(
        "items",
        vec![
            make_prop("id", "i32", false, false),
            make_prop("sku", "String", false, true), // unique index
        ],
    );
    let migration = engine.generate("InitialItems", &[meta], &None).unwrap();
    engine.apply(&*provider, &migration).await.unwrap();

    // Verify the index exists by querying sqlite_master
    let mut conn = provider.get_connection().await.unwrap();
    let rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='items'",
            &[],
        )
        .await
        .unwrap();
    let index_names: Vec<String> = rows
        .into_iter()
        .filter_map(|row| row.into_iter().next())
        .filter_map(|v| String::try_from(v).ok())
        .collect();

    assert!(
        index_names.iter().any(|n| n.contains("ix_items_sku")),
        "Expected ix_items_sku in indexes: {:?}",
        index_names
    );
}
