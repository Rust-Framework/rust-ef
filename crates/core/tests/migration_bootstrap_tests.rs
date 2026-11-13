//! History-free bootstrap (EF-8).
//!
//! A code-first install must be able to create the whole schema on an empty
//! database without creating `__ef_migrations_history`: the history table is
//! the migration bookkeeping, and the installer owns the "database is
//! installed" marker instead.

use rust_ef::entity::IEntityType;
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::provider::IDatabaseProvider;
use rust_ef::EntityType;

#[derive(Debug, Clone, EntityType)]
#[table("boot_users")]
struct BootUser {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[max_length(64)]
    pub email: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("boot_orders")]
struct BootOrder {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[foreign_key(BootUser)]
    pub user_id: i32,
    pub total: i64,
}

async fn table_names(provider: &dyn IDatabaseProvider) -> Vec<String> {
    let mut conn = provider.get_connection().await.expect("connection");
    let rows = conn
        .query("SELECT name FROM sqlite_master WHERE type='table'", &[])
        .await
        .expect("sqlite_master");
    rows.iter()
        .map(|row| String::try_from(row[0].clone()).expect("table name"))
        .collect()
}

#[tokio::test]
async fn bootstrap_creates_all_tables_without_creating_history() {
    let provider = std::sync::Arc::new(rust_ef_sqlite::SqliteProvider::new_in_memory().unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    engine
        .bootstrap(&*provider, &[BootUser::entity_meta(), BootOrder::entity_meta()])
        .await
        .expect("bootstrap an empty database");

    let tables = table_names(&*provider).await;
    assert!(tables.iter().any(|t| t == "boot_users"), "tables: {tables:?}");
    assert!(
        tables.iter().any(|t| t == "boot_orders"),
        "tables: {tables:?}"
    );
    assert!(
        !tables.iter().any(|t| t == "__ef_migrations_history"),
        "bootstrap must not create the history table: {tables:?}"
    );

    // Bootstrap emits the full schema: the FK is part of the SQLite DDL.
    let mut conn = provider.get_connection().await.unwrap();
    let ddl = conn
        .query(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='boot_orders'",
            &[],
        )
        .await
        .unwrap();
    let ddl = String::try_from(ddl[0][0].clone()).unwrap();
    assert!(
        ddl.contains("FOREIGN KEY"),
        "bootstrap must include foreign keys:\n{ddl}"
    );
}

#[tokio::test]
async fn bootstrap_does_not_record_a_migration_or_need_history() {
    let provider = std::sync::Arc::new(rust_ef_sqlite::SqliteProvider::new_in_memory().unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    engine
        .bootstrap(&*provider, &[BootUser::entity_meta()])
        .await
        .expect("bootstrap");

    // The history table does not exist, so asking for applied migrations must
    // create it lazily and report nothing applied — it must not error.
    let applied = engine
        .get_applied_migrations(&*provider)
        .await
        .expect("history lookup on a bootstrapped database");
    assert!(applied.is_empty(), "bootstrap is not a migration: {applied:?}");
}

#[tokio::test]
async fn bootstrap_rejects_an_empty_model() {
    let provider = std::sync::Arc::new(rust_ef_sqlite::SqliteProvider::new_in_memory().unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let err = engine.bootstrap(&*provider, &[]).await;
    assert!(err.is_err(), "bootstrap without entity types must fail");
}
