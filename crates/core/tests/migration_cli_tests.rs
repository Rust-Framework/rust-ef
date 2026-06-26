//! Integration tests for `MigrationEngine::revert_to_target` and
//! `MigrationEngine::generate_script` — the two APIs backing the CLI's
//! `migration revert --target` and `migration script --from/--to` commands.

use rust_ef::migration::{Migration, MigrationDialect, MigrationEngine};
use rust_ef_sqlite::SqliteProvider;
use std::sync::Arc;

/// Builds a hand-crafted migration that creates/drops a table named after the
/// migration id. The `{migration_id}` placeholder is preserved so the engine
/// can substitute it during apply/revert (matching `MigrationEngine::generate`
/// output format).
fn make_migration(id: &str, table: &str) -> Migration {
    Migration {
        id: id.to_string(),
        description: id.to_string(),
        up_sql: format!(
            "CREATE TABLE \"{table}\" (\"id\" INTEGER NOT NULL PRIMARY KEY);\n\
             INSERT INTO \"__ef_migrations_history\"(\"migration_id\", \"product_version\") \
             VALUES ('{{migration_id}}', 'test');\n"
        ),
        down_sql: format!(
            "DROP TABLE IF EXISTS \"{table}\";\n\
             DELETE FROM \"__ef_migrations_history\" WHERE migration_id = '{{migration_id}}';\n"
        ),
    }
}

fn three_migrations() -> Vec<Migration> {
    vec![
        make_migration("0001_Init", "t1"),
        make_migration("0002_AddCol", "t2"),
        make_migration("0003_AddIdx", "t3"),
    ]
}

async fn setup_provider_with_all(migrations: &[Migration]) -> Arc<SqliteProvider> {
    let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    engine.ensure_history_table(&*provider).await.unwrap();
    for m in migrations {
        engine.apply(&*provider, m).await.unwrap();
    }
    provider
}

// ---------------------------------------------------------------------------
// revert_to_target
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_revert_to_target_reverts_only_after_target() {
    let migrations = three_migrations();
    let provider = setup_provider_with_all(&migrations).await;
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let applied_before = engine.get_applied_migrations(&*provider).await.unwrap();
    assert_eq!(applied_before.len(), 3);

    let reverted = engine
        .revert_to_target(&*provider, &migrations, Some("0002_AddCol"))
        .await
        .unwrap();

    assert_eq!(reverted, vec!["0003_AddIdx".to_string()]);

    let applied_after = engine.get_applied_migrations(&*provider).await.unwrap();
    assert_eq!(applied_after.len(), 2);
    assert!(engine.is_applied(&*provider, "0002_AddCol").await.unwrap());
    assert!(!engine.is_applied(&*provider, "0003_AddIdx").await.unwrap());
}

#[tokio::test]
async fn test_revert_to_target_none_reverts_all() {
    let migrations = three_migrations();
    let provider = setup_provider_with_all(&migrations).await;
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let reverted = engine
        .revert_to_target(&*provider, &migrations, None)
        .await
        .unwrap();

    assert_eq!(reverted.len(), 3);
    // Reverted in reverse order (most-recent first).
    assert_eq!(reverted[0], "0003_AddIdx");
    assert_eq!(reverted[1], "0002_AddCol");
    assert_eq!(reverted[2], "0001_Init");

    let applied = engine.get_applied_migrations(&*provider).await.unwrap();
    assert!(applied.is_empty());
}

#[tokio::test]
async fn test_revert_to_target_at_last_yields_empty() {
    let migrations = three_migrations();
    let provider = setup_provider_with_all(&migrations).await;
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let reverted = engine
        .revert_to_target(&*provider, &migrations, Some("0003_AddIdx"))
        .await
        .unwrap();

    assert!(reverted.is_empty());
    assert!(engine.is_applied(&*provider, "0003_AddIdx").await.unwrap());
}

#[tokio::test]
async fn test_revert_to_target_not_applied_errors() {
    let migrations = three_migrations();
    let provider = setup_provider_with_all(&migrations).await;
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let result = engine
        .revert_to_target(&*provider, &migrations, Some("9999_Nonexistent"))
        .await;

    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// generate_script
// ---------------------------------------------------------------------------

#[test]
fn test_generate_script_forward_range() {
    let migrations = three_migrations();
    let sql = MigrationEngine::generate_script(&migrations, Some("0001_Init"), Some("0003_AddIdx"))
        .unwrap();

    assert!(sql.contains("forward"));
    assert!(sql.contains("-- Up: 0002_AddCol"));
    assert!(sql.contains("-- Up: 0003_AddIdx"));
    // 0001 is the `from` (exclusive) — its up SQL should NOT appear.
    assert!(!sql.contains("-- Up: 0001_Init"));
}

#[test]
fn test_generate_script_reverse_range() {
    let migrations = three_migrations();
    let sql = MigrationEngine::generate_script(&migrations, Some("0003_AddIdx"), Some("0001_Init"))
        .unwrap();

    assert!(sql.contains("reverse"));
    assert!(sql.contains("-- Down: 0003_AddIdx"));
    assert!(sql.contains("-- Down: 0002_AddCol"));
    // 0001 is the `to` (inclusive boundary) — its down SQL should NOT appear.
    assert!(!sql.contains("-- Down: 0001_Init"));
}

#[test]
fn test_generate_script_from_none_to_specific() {
    let migrations = three_migrations();
    let sql = MigrationEngine::generate_script(&migrations, None, Some("0002_AddCol")).unwrap();

    assert!(sql.contains("-- Up: 0001_Init"));
    assert!(sql.contains("-- Up: 0002_AddCol"));
    assert!(!sql.contains("0003_AddIdx"));
}

#[test]
fn test_generate_script_from_specific_to_none() {
    let migrations = three_migrations();
    let sql = MigrationEngine::generate_script(&migrations, Some("0001_Init"), None).unwrap();

    assert!(!sql.contains("-- Up: 0001_Init"));
    assert!(sql.contains("-- Up: 0002_AddCol"));
    assert!(sql.contains("-- Up: 0003_AddIdx"));
}

#[test]
fn test_generate_script_from_none_to_none() {
    let migrations = three_migrations();
    let sql = MigrationEngine::generate_script(&migrations, None, None).unwrap();

    // From empty DB to latest: all up scripts.
    assert!(sql.contains("-- Up: 0001_Init"));
    assert!(sql.contains("-- Up: 0002_AddCol"));
    assert!(sql.contains("-- Up: 0003_AddIdx"));
}

#[test]
fn test_generate_script_from_equals_to_noop() {
    let migrations = three_migrations();
    let sql =
        MigrationEngine::generate_script(&migrations, Some("0002_AddCol"), Some("0002_AddCol"))
            .unwrap();

    assert!(sql.contains("Nothing to do"));
}

#[test]
fn test_generate_script_unknown_from_errors() {
    let migrations = three_migrations();
    let result = MigrationEngine::generate_script(&migrations, Some("9999_Nope"), None);
    assert!(result.is_err());
}

#[test]
fn test_generate_script_unknown_to_errors() {
    let migrations = three_migrations();
    let result = MigrationEngine::generate_script(&migrations, None, Some("9999_Nope"));
    assert!(result.is_err());
}

#[test]
fn test_generate_script_empty_migrations_noop() {
    let migrations: Vec<Migration> = Vec::new();
    let sql = MigrationEngine::generate_script(&migrations, None, None).unwrap();
    // from_idx=-1, to_idx=-1 (len-1 when empty = -1): from==to → no-op.
    assert!(sql.contains("Nothing to do"));
}
