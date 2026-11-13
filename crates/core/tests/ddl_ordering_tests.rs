//! DDL dependency ordering (EF-10).
//!
//! MySQL rejects `ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY` when the
//! referenced column is not indexed yet (error 1822). The generator therefore
//! emits `CREATE TABLE` → `CREATE INDEX` → `ADD CONSTRAINT` regardless of the
//! order the diff produced; callers must not have to reorder the script.

use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::model_builder::{EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder};
use rust_ef::provider::IDatabaseProvider;
use rust_ef::EntityType;

#[derive(Debug, Clone, EntityType)]
#[table("ord_parents")]
struct OrdParent {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    /// Unique column: the generated unique index is the FK target.
    #[max_length(64)]
    #[unique]
    pub code: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("ord_children")]
struct OrdChild {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[max_length(64)]
    pub parent_code: String,
}

#[derive(Default)]
struct OrdChildConfig;

impl IEntityTypeConfiguration<OrdChild> for OrdChildConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, OrdChild>) {
        entity.has_foreign_key(
            &[OrdChild::COLUMN_PARENT_CODE],
            OrdParent::TABLE,
            &[OrdParent::COLUMN_CODE],
        );
    }
}

fn model() -> Vec<rust_ef::metadata::EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.entity::<OrdParent>();
    builder.apply_configuration::<OrdChildConfig, OrdChild>();
    builder.build()
}

fn generated_up_sql(dialect: MigrationDialect) -> String {
    MigrationEngine::new(dialect)
        .generate("InitialCreate", &model(), &None)
        .expect("generate")
        .up_sql
}

#[test]
fn referenced_index_is_created_before_the_foreign_key() {
    // SQLite emits foreign keys inline in CREATE TABLE, so it has no
    // `ALTER TABLE ADD CONSTRAINT` to order against.
    for dialect in [MigrationDialect::MySql, MigrationDialect::Postgres] {
        let sql = generated_up_sql(dialect);
        let q = if dialect == MigrationDialect::MySql {
            "`"
        } else {
            "\""
        };
        let index = format!("CREATE UNIQUE INDEX {q}ix_ord_parents_code{q}");
        let foreign_key = format!(
            "ALTER TABLE {q}ord_children{q} ADD CONSTRAINT {q}fk_ord_children_parent_code_ord_parents{q}"
        );
        let index_pos = sql
            .find(&index)
            .unwrap_or_else(|| panic!("{dialect:?} script is missing the referenced index:\n{sql}"));
        let fk_pos = sql
            .find(&foreign_key)
            .unwrap_or_else(|| panic!("{dialect:?} script is missing the FK constraint:\n{sql}"));
        assert!(
            index_pos < fk_pos,
            "{dialect:?} must create the referenced unique index before the FK (index at {index_pos}, FK at {fk_pos}):\n{sql}"
        );
    }
}

#[test]
fn every_table_exists_before_indexes_and_constraints() {
    for dialect in [
        MigrationDialect::Sqlite,
        MigrationDialect::MySql,
        MigrationDialect::Postgres,
    ] {
        let sql = generated_up_sql(dialect);
        let last_create_table = sql
            .rfind("CREATE TABLE")
            .unwrap_or_else(|| panic!("{dialect:?} script has no CREATE TABLE:\n{sql}"));
        let first_index = sql
            .find("CREATE UNIQUE INDEX")
            .unwrap_or_else(|| panic!("{dialect:?} script has no unique index:\n{sql}"));
        assert!(
            last_create_table < first_index,
            "{dialect:?} must finish CREATE TABLE before CREATE INDEX:\n{sql}"
        );
        if let Some(first_constraint) = sql.find("ADD CONSTRAINT") {
            assert!(
                last_create_table < first_constraint,
                "{dialect:?} must finish CREATE TABLE before ADD CONSTRAINT:\n{sql}"
            );
        }
    }
}

#[tokio::test]
async fn sqlite_executes_the_generated_script_without_reordering() {
    let provider = std::sync::Arc::new(rust_ef_sqlite::SqliteProvider::new_in_memory().unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let migration = engine.generate("InitialCreate", &model(), &None).unwrap();

    engine
        .apply(&*provider, &migration)
        .await
        .expect("generated script must execute as-is");

    let mut conn = provider.get_connection().await.unwrap();
    let rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
            &[],
        )
        .await
        .unwrap();
    let names: Vec<String> = rows
        .iter()
        .map(|row| String::try_from(row[0].clone()).unwrap())
        .collect();
    assert!(names.iter().any(|n| n == "ord_parents"), "tables: {names:?}");
    assert!(names.iter().any(|n| n == "ord_children"), "tables: {names:?}");
}
