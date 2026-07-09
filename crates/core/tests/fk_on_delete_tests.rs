//! Integration tests for FK `ON DELETE` clause generation in migration DDL.
//!
//! Verifies that `MigrationEngine::generate` emits `ON DELETE {CASCADE|RESTRICT|
//! SET NULL|NO ACTION}` clauses based on:
//! - Default behavior for required FKs (non-nullable `i32`) → CASCADE
//! - Default behavior for optional FKs (nullable `Option<i32>`) → RESTRICT
//! - Explicit `#[on_delete(SetNull)]` → SET NULL
//! - Explicit `#[on_delete(NoAction)]` → NO ACTION
//! - Diff detection when ON DELETE clause changes between snapshots
//! - JSON serialization roundtrip of `fk_on_delete` field

use rust_ef::entity::IEntityType;
use rust_ef::migration::{MigrationDialect, MigrationEngine, MigrationStore};
use rust_ef::prelude::*;

// ---------------------------------------------------------------------------
// Test entities
// ---------------------------------------------------------------------------

/// Principal with required FK (i32) → default CASCADE
#[derive(Debug, Clone, EntityType)]
#[table("fk_cascade_blogs")]
#[allow(dead_code)]
struct FkCascadeBlog {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[navigation]
    posts: HasMany<FkCascadePost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("fk_cascade_posts")]
#[allow(dead_code)]
struct FkCascadePost {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[foreign_key(FkCascadeBlog)]
    blog_id: i32,
    #[navigation]
    blog: BelongsTo<FkCascadeBlog>,
}

/// Principal with optional FK (Option<i32>) → default RESTRICT
#[derive(Debug, Clone, EntityType)]
#[table("fk_optional_blogs")]
#[allow(dead_code)]
struct FkOptionalBlog {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[navigation]
    posts: HasMany<FkOptionalPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("fk_optional_posts")]
#[allow(dead_code)]
struct FkOptionalPost {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[foreign_key(FkOptionalBlog)]
    blog_id: Option<i32>,
    #[navigation]
    blog: BelongsTo<FkOptionalBlog>,
}

/// Principal with explicit #[on_delete(SetNull)] → SET NULL
#[derive(Debug, Clone, EntityType)]
#[table("fk_setnull_blogs")]
#[allow(dead_code)]
struct FkSetNullBlog {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[navigation]
    #[on_delete(SetNull)]
    posts: HasMany<FkSetNullPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("fk_setnull_posts")]
#[allow(dead_code)]
struct FkSetNullPost {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[foreign_key(FkSetNullBlog)]
    blog_id: Option<i32>,
    #[navigation]
    blog: BelongsTo<FkSetNullBlog>,
}

/// Principal with explicit #[on_delete(NoAction)] → NO ACTION
#[derive(Debug, Clone, EntityType)]
#[table("fk_noaction_blogs")]
#[allow(dead_code)]
struct FkNoActionBlog {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[navigation]
    #[on_delete(NoAction)]
    posts: HasMany<FkNoActionPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("fk_noaction_posts")]
#[allow(dead_code)]
struct FkNoActionPost {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[foreign_key(FkNoActionBlog)]
    blog_id: i32,
    #[navigation]
    blog: BelongsTo<FkNoActionBlog>,
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn assert_fk_on_delete(up_sql: &str, expected_clause: &str, label: &str) {
    assert!(
        up_sql.contains(&format!("ON DELETE {}", expected_clause)),
        "{}: up_sql should contain 'ON DELETE {}':\n{}",
        label,
        expected_clause,
        up_sql
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_required_fk_generates_cascade() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let metas = vec![
        FkCascadeBlog::entity_meta(),
        FkCascadePost::entity_meta(),
    ];
    let migration = engine.generate("InitCascade", &metas, &None).unwrap();

    assert_fk_on_delete(&migration.up_sql, "CASCADE", "required FK");

    // Verify the FK constraint targets the correct table/column
    assert!(
        migration.up_sql.contains("\"fk_cascade_posts\""),
        "up_sql should reference fk_cascade_posts table"
    );
    assert!(
        migration.up_sql.contains("\"blog_id\""),
        "up_sql should reference blog_id column"
    );
}

#[test]
fn test_optional_fk_generates_restrict() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let metas = vec![
        FkOptionalBlog::entity_meta(),
        FkOptionalPost::entity_meta(),
    ];
    let migration = engine.generate("InitRestrict", &metas, &None).unwrap();

    assert_fk_on_delete(&migration.up_sql, "RESTRICT", "optional FK");
}

#[test]
fn test_explicit_set_null() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let metas = vec![
        FkSetNullBlog::entity_meta(),
        FkSetNullPost::entity_meta(),
    ];
    let migration = engine.generate("InitSetNull", &metas, &None).unwrap();

    assert_fk_on_delete(&migration.up_sql, "SET NULL", "explicit SetNull");
}

#[test]
fn test_explicit_no_action() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let metas = vec![
        FkNoActionBlog::entity_meta(),
        FkNoActionPost::entity_meta(),
    ];
    let migration = engine.generate("InitNoAction", &metas, &None).unwrap();

    assert_fk_on_delete(&migration.up_sql, "NO ACTION", "explicit NoAction");
}

#[test]
fn test_on_delete_change_triggers_fk_rebuild() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Build current snapshot from FkCascadePost (CASCADE)
    let current_metas = vec![
        FkCascadeBlog::entity_meta(),
        FkCascadePost::entity_meta(),
    ];
    let current_snapshot = engine.create_snapshot("current", &current_metas);

    // Build previous snapshot manually: same structure but SET NULL on blog_id
    let mut prev_snapshot = current_snapshot.clone();
    prev_snapshot.migration_id = "previous".to_string();
    for et in &mut prev_snapshot.entity_types {
        if et.table_name == "fk_cascade_posts" {
            for col in &mut et.columns {
                if col.column_name == "blog_id" {
                    col.fk_on_delete = Some("SET NULL".to_string());
                }
            }
        }
    }

    // Generate migration: diff should detect ON DELETE change
    let migration = engine
        .generate("ChangeOnDelete", &current_metas, &Some(prev_snapshot))
        .unwrap();

    // Should drop the old FK and add a new one with CASCADE
    assert!(
        migration.up_sql.contains("DROP CONSTRAINT") || migration.up_sql.contains("DROP FOREIGN KEY"),
        "up_sql should contain DROP CONSTRAINT/FOREIGN KEY:\n{}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("ADD CONSTRAINT"),
        "up_sql should contain ADD CONSTRAINT:\n{}",
        migration.up_sql
    );
    assert_fk_on_delete(&migration.up_sql, "CASCADE", "ON DELETE change");
}

#[test]
fn test_snapshot_json_roundtrip() {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let metas = vec![
        FkCascadeBlog::entity_meta(),
        FkCascadePost::entity_meta(),
    ];
    let snapshot = engine.create_snapshot("roundtrip", &metas);

    // Verify fk_on_delete is populated
    let posts_et = snapshot
        .entity_types
        .iter()
        .find(|et| et.table_name == "fk_cascade_posts")
        .expect("fk_cascade_posts should be in snapshot");
    let blog_id_col = posts_et
        .columns
        .iter()
        .find(|c| c.column_name == "blog_id")
        .expect("blog_id column should exist");
    assert_eq!(
        blog_id_col.fk_on_delete.as_deref(),
        Some("CASCADE"),
        "blog_id should have fk_on_delete = CASCADE"
    );

    // Save to temp dir and reload
    let tmp = std::env::temp_dir().join("fk_on_delete_roundtrip_test");
    let store = MigrationStore::new(&tmp);
    store.save_snapshot(&snapshot).unwrap();

    // Verify JSON contains fk_on_delete
    let json_text = std::fs::read_to_string(tmp.join("model_snapshot.json")).unwrap();
    assert!(
        json_text.contains("\"fk_on_delete\":\"CASCADE\""),
        "JSON should contain fk_on_delete field:\n{}",
        json_text
    );

    // Reload and verify
    let loaded = store.load_snapshot().unwrap().expect("snapshot should load");
    let loaded_posts = loaded
        .entity_types
        .iter()
        .find(|et| et.table_name == "fk_cascade_posts")
        .expect("fk_cascade_posts should be in loaded snapshot");
    let loaded_blog_id = loaded_posts
        .columns
        .iter()
        .find(|c| c.column_name == "blog_id")
        .expect("blog_id column should exist in loaded snapshot");
    assert_eq!(
        loaded_blog_id.fk_on_delete.as_deref(),
        Some("CASCADE"),
        "loaded blog_id should have fk_on_delete = CASCADE"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}
