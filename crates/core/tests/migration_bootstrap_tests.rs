//! History-free bootstrap (EF-8).
//!
//! A code-first install must be able to create the whole schema on an empty
//! database without creating `__ef_migrations_history`: the history table is
//! the migration bookkeeping, and the installer owns the "database is
//! installed" marker instead.

use rust_ef::entity::IEntityType;
use rust_ef::metadata::EntityTypeMeta;
use rust_ef::migration::{MigrationDialect, MigrationEngine, SeedData};
use rust_ef::model_builder::ModelBuilder;
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

// ---------------------------------------------------------------------------
// EF-11: `has_data` seeds are part of the bootstrap path
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, EntityType)]
#[table("boot_roles")]
struct BootRole {
    #[primary_key]
    #[max_length(32)]
    pub code: String,
    #[max_length(64)]
    pub name: String,
}

fn role_seeds<'a>(
    builder: &'a ModelBuilder,
    metas: &'a [EntityTypeMeta],
) -> Vec<SeedData<'a>> {
    metas
        .iter()
        .filter(|meta| !meta.is_view)
        .filter_map(|meta| {
            let rows = builder.seed_rows_for(&meta.type_id);
            (!rows.is_empty()).then_some(SeedData { meta, rows })
        })
        .collect()
}

#[tokio::test]
async fn bootstrap_with_seed_inserts_model_declared_rows() {
    let provider = std::sync::Arc::new(rust_ef_sqlite::SqliteProvider::new_in_memory().unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let mut builder = ModelBuilder::new();
    builder.entity::<BootRole>().has_data(&[
        BootRole {
            code: "admin".into(),
            name: "Admin".into(),
        },
        BootRole {
            code: "viewer".into(),
            name: "Viewer".into(),
        },
    ]);
    let metas = builder.build();
    let seeds = role_seeds(&builder, &metas);
    assert_eq!(seeds.len(), 1, "one entity type declares seed rows");

    engine
        .bootstrap_with_seed(&*provider, &metas, &seeds)
        .await
        .expect("bootstrap → seed");

    let mut conn = provider.get_connection().await.unwrap();
    let rows = conn
        .query("SELECT code FROM boot_roles ORDER BY code", &[])
        .await
        .unwrap();
    let codes: Vec<String> = rows
        .iter()
        .map(|row| String::try_from(row[0].clone()).unwrap())
        .collect();
    assert_eq!(codes, vec!["admin", "viewer"]);
}

#[tokio::test]
async fn dbcontext_bootstrap_runs_bootstrap_then_seed_in_one_call() {
    use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
    use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build()).expect("context");
    ctx.model().entity::<BootRole>().has_data(&[BootRole {
        code: "admin".into(),
        name: "Admin".into(),
    }]);

    ctx.bootstrap().await.expect("DbContext::bootstrap");

    let mut conn = ctx.provider().get_connection().await.unwrap();
    let rows = conn
        .query("SELECT COUNT(*) FROM boot_roles", &[])
        .await
        .unwrap();
    assert_eq!(i64::try_from(rows[0][0].clone()).unwrap(), 1);

    // The one-call install path must not create migration bookkeeping.
    let tables = table_names(&*ctx.provider()).await;
    assert!(
        !tables.iter().any(|t| t == "__ef_migrations_history"),
        "tables: {tables:?}"
    );
}

