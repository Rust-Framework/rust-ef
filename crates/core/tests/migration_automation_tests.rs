//! Automated upgrade workflow: add a column in the next iteration, generate a
//! migration from the snapshot diff, and apply it without hand-written SQL.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::IEntityType;
use rust_ef::migration::{Migration, MigrationDialect, MigrationEngine, MigrationStore};
use rust_ef::model_builder::{EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder};
use rust_ef::provider::{DbValue, IDatabaseProvider};
use rust_ef::EntityType;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("widgets")]
struct WidgetV1 {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("widgets")]
struct WidgetV2 {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub name: String,
    pub stock: i32,
    #[column_type("VARCHAR(64)")]
    pub note: Option<String>,
}

#[derive(Default)]
struct WidgetV2Config;

impl IEntityTypeConfiguration<WidgetV2> for WidgetV2Config {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, WidgetV2>) {
        // Default makes ADD COLUMN safe on a non-empty table.
        entity.property_named("stock").has_default("0");
    }
}

fn v2_model() -> Vec<rust_ef::metadata::EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.apply_configuration::<WidgetV2Config, WidgetV2>();
    builder.build()
}

#[tokio::test]
async fn add_column_upgrade_is_generated_and_applied() {
    let provider = std::sync::Arc::new(rust_ef_sqlite::SqliteProvider::new(":memory:").unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Iteration 1: initial schema.
    let v1 = engine
        .generate("Initial", &[WidgetV1::entity_meta()], &None)
        .unwrap();
    engine.apply(&*provider, &v1).await.unwrap();
    let mut conn = provider.get_connection().await.unwrap();
    conn.execute(
        "INSERT INTO widgets (name) VALUES (?)",
        &[DbValue::String("old row".into())],
    )
    .await
    .unwrap();

    // Iteration 2: add `stock` (non-Option → NOT NULL, default 0) + nullable `note`.
    let v1_snapshot = engine.create_snapshot("v1", &[WidgetV1::entity_meta()]);
    let upgrade = engine
        .generate("AddStockAndNote", &v2_model(), &Some(v1_snapshot))
        .unwrap();
    assert!(
        upgrade
            .up_sql
            .contains("ALTER TABLE \"widgets\" ADD COLUMN \"stock\" INTEGER NOT NULL DEFAULT 0"),
        "expected ADD COLUMN with default:\n{}",
        upgrade.up_sql
    );
    assert!(
        upgrade.up_sql.contains("\"note\" VARCHAR(64)"),
        "expected typed nullable column:\n{}",
        upgrade.up_sql
    );

    engine.apply(&*provider, &upgrade).await.unwrap();

    // Existing rows get the default; the new nullable column is NULL.
    let rows = conn
        .query("SELECT stock, note FROM widgets", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(i64::try_from(rows[0][0].clone()).unwrap(), 0);
    assert_eq!(rows[0][1], DbValue::Null);

    // Re-applying is a no-op (history guard).
    engine.apply(&*provider, &upgrade).await.unwrap();
    let rows = conn
        .query("SELECT COUNT(*) FROM widgets", &[])
        .await
        .unwrap();
    assert_eq!(rows[0][0], DbValue::I64(1));
}

#[tokio::test]
async fn migration_script_is_atomic_on_sqlite() {
    let provider = std::sync::Arc::new(rust_ef_sqlite::SqliteProvider::new(":memory:").unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let broken = Migration {
        id: "Broken".into(),
        description: "Broken".into(),
        up_sql: "CREATE TABLE tx_probe (id INTEGER PRIMARY KEY);\n\
                 CREATE TABLE tx_probe (id INTEGER PRIMARY KEY);\n"
            .into(),
        down_sql: String::new(),
    };
    let err = engine.apply(&*provider, &broken).await;
    assert!(err.is_err(), "duplicate CREATE TABLE must fail");

    // SQLite DDL is transactional: the first statement must be rolled back.
    let mut conn = provider.get_connection().await.unwrap();
    let rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='tx_probe'",
            &[],
        )
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "failed migration must not leave a partially-created table"
    );
}

#[test]
fn migration_store_rejects_unsafe_ids() {
    let dir = std::env::temp_dir().join("rust_ef_unsafe_id");
    let store = MigrationStore::new(&dir);
    for id in ["../evil", "a/b", "a\\b", "C:evil", ".", ".."] {
        let migration = Migration {
            id: id.into(),
            description: String::new(),
            up_sql: String::new(),
            down_sql: String::new(),
        };
        assert!(
            store.save(&migration).is_err(),
            "id {id:?} must be rejected"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn dbcontext_migrate_applies_pending_migrations() {
    let dir = std::env::temp_dir().join("rust_ef_migrate_dir");
    let _ = std::fs::remove_dir_all(&dir);
    let store = MigrationStore::new(&dir);
    store
        .save(&Migration {
            id: "Initial".into(),
            description: "Initial".into(),
            up_sql: "CREATE TABLE migrate_probe (id INTEGER PRIMARY KEY, name TEXT);\n\
                     INSERT INTO __ef_migrations_history(migration_id, product_version) \
                     VALUES ('{migration_id}', 'test');\n"
                .into(),
            down_sql:
                "DROP TABLE IF EXISTS migrate_probe;\n\
                       DELETE FROM __ef_migrations_history WHERE migration_id = '{migration_id}';\n"
                    .into(),
        })
        .unwrap();

    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let ctx = DbContext::from_options(&options).unwrap();

    assert_eq!(ctx.migrate(&dir).await.unwrap(), 1, "first run applies");
    assert_eq!(ctx.migrate(&dir).await.unwrap(), 0, "second run is a no-op");

    let mut conn = ctx.provider().get_connection().await.unwrap();
    let rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='migrate_probe'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "migration table created");

    let _ = std::fs::remove_dir_all(&dir);
}
