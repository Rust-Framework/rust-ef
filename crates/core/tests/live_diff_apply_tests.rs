//! End-to-end runtime model diff + apply against a real SQLite database (EF-9).
//!
//! The live schema is read straight from SQLite (`sqlite_master` + PRAGMAs) so
//! these tests exercise the same input shape `IDatabaseProvider::introspect`
//! will produce (EF-5). The helper stays in the test file until that API lands
//! on the branch.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rust_ef::entity::IEntityType;
use rust_ef::metadata::EntityTypeMeta;
use rust_ef::migration::{MigrationDialect, MigrationDiff, MigrationEngine};
use rust_ef::model_builder::{EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder};
use rust_ef::provider::{DbValue, IDatabaseProvider};
use rust_ef::schema::{
    DbCheck, DbColumn, DbForeignKey, DbIndex, DbIndexColumn, DbTable, DbTrigger,
};
use rust_ef::EntityType;
use rust_ef_sqlite::SqliteProvider;

// ---------------------------------------------------------------------------
// Schema reader (test-only stand-in for EF-5 introspection)
// ---------------------------------------------------------------------------

/// Reads every user table (skipping SQLite internals and EF history) including
/// columns, explicit indexes, foreign keys, CHECK constraints and triggers.
async fn read_sqlite_schema(provider: &dyn IDatabaseProvider) -> Vec<DbTable> {
    let mut connection = provider.get_connection().await.expect("connection");
    let table_rows = connection
        .query(
            "SELECT name, sql FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '__ef_%' \
             ORDER BY name",
            &[],
        )
        .await
        .expect("list tables");

    let mut tables = Vec::new();
    for table_row in &table_rows {
        let name = string_value(&table_row[0]);
        let create_sql = optional_string_value(&table_row[1]);
        let mut table = DbTable::new(&name);

        // --- columns -------------------------------------------------------
        let column_rows = connection
            .query(&format!("PRAGMA table_info({})", quoted(&name)), &[])
            .await
            .expect("table_info");
        let primary_key_count = column_rows
            .iter()
            .filter(|row| int_value(&row[5]) > 0)
            .count();
        for row in &column_rows {
            let column_name = string_value(&row[1]);
            let data_type = optional_string_value(&row[2]).unwrap_or_default();
            let not_null = int_value(&row[3]) != 0;
            let default_value = optional_string_value(&row[4]);
            let is_primary_key = int_value(&row[5]) > 0;
            table.columns.push(DbColumn {
                name: column_name,
                data_type: data_type.clone(),
                is_nullable: !not_null && !is_primary_key,
                is_primary_key,
                is_auto_increment: is_primary_key
                    && primary_key_count == 1
                    && data_type.eq_ignore_ascii_case("INTEGER"),
                max_length: parse_type_length(&data_type),
                default_value,
                ..DbColumn::default()
            });
        }

        // --- indexes (explicit CREATE INDEX only) --------------------------
        let index_rows = connection
            .query(&format!("PRAGMA index_list({})", quoted(&name)), &[])
            .await
            .expect("index_list");
        for row in &index_rows {
            let origin = optional_string_value(&row[3]).unwrap_or_default();
            if origin != "c" {
                continue;
            }
            let index_name = string_value(&row[1]);
            let is_unique = int_value(&row[2]) != 0;
            let info_rows = connection
                .query(&format!("PRAGMA index_info({})", quoted(&index_name)), &[])
                .await
                .expect("index_info");
            let columns = info_rows
                .iter()
                .filter_map(|info| {
                    optional_string_value(&info[2]).map(|value| DbIndexColumn {
                        value,
                        is_expression: false,
                    })
                })
                .collect();
            table.indexes.push(DbIndex {
                name: index_name,
                columns,
                is_unique,
                filter: None,
            });
        }

        // --- foreign keys --------------------------------------------------
        let foreign_key_rows = connection
            .query(&format!("PRAGMA foreign_key_list({})", quoted(&name)), &[])
            .await
            .expect("foreign_key_list");
        type ForeignKeyRow = (i64, String, String, Option<String>, Option<String>);
        let mut grouped: BTreeMap<i64, Vec<ForeignKeyRow>> = BTreeMap::new();
        for row in &foreign_key_rows {
            let id = int_value(&row[0]);
            let seq = int_value(&row[1]);
            let referenced_table = string_value(&row[2]);
            let from = string_value(&row[3]);
            let to = optional_string_value(&row[4]).filter(|to| !to.is_empty());
            let on_delete = optional_string_value(&row[6]);
            grouped
                .entry(id)
                .or_default()
                .push((seq, from, referenced_table, to, on_delete));
        }
        for (_, mut parts) in grouped {
            parts.sort_by_key(|part| part.0);
            let columns: Vec<String> = parts.iter().map(|part| part.1.clone()).collect();
            let referenced_table = parts[0].2.clone();
            let referenced_columns: Vec<String> = parts
                .iter()
                .map(|part| part.3.clone().unwrap_or_default())
                .collect();
            let on_delete = parts[0].4.clone();
            table.foreign_keys.push(DbForeignKey {
                name: None,
                columns,
                referenced_table,
                referenced_columns,
                on_delete: on_delete.filter(|action| action != "NO ACTION"),
                ..DbForeignKey::default()
            });
        }

        // --- triggers ------------------------------------------------------
        let trigger_rows = connection
            .query(
                "SELECT name, sql FROM sqlite_master WHERE type = 'trigger' AND tbl_name = ?",
                &[DbValue::String(name.clone())],
            )
            .await
            .expect("list triggers");
        table.triggers = Some(
            trigger_rows
                .iter()
                .filter_map(|row| {
                    let trigger_name = optional_string_value(&row[0])?;
                    let sql = optional_string_value(&row[1])?;
                    Some(DbTrigger {
                        name: trigger_name,
                        sql,
                    })
                })
                .collect(),
        );

        // --- CHECK constraints (parsed from the table DDL) -----------------
        table.checks = create_sql.as_deref().map(parse_checks);

        tables.push(table);
    }
    tables
}

/// Extracts table-level `CHECK (...)` expressions from a `CREATE TABLE` script.
fn parse_checks(create_sql: &str) -> Vec<DbCheck> {
    let lower = create_sql.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut checks = Vec::new();
    let mut position = 0usize;
    while let Some(offset) = lower[position..].find("check") {
        let start = position + offset;
        position = start + "check".len();
        let boundary_ok = start == 0 || {
            let before = bytes[start - 1] as char;
            !before.is_ascii_alphanumeric() && before != '_'
        };
        if !boundary_ok {
            continue;
        }
        let rest = create_sql[position..].trim_start();
        if !rest.starts_with('(') {
            continue;
        }
        let open = create_sql[position..]
            .find('(')
            .map(|index| position + index);
        let Some(open) = open else { continue };
        let mut depth = 0usize;
        let mut end = None;
        for (index, character) in create_sql[open..].char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + index);
                        break;
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            checks.push(DbCheck {
                name: None,
                expression: create_sql[open + 1..end].trim().to_string(),
            });
            position = end + 1;
        }
    }
    checks
}

fn string_value(value: &DbValue) -> String {
    String::try_from(value.clone()).unwrap_or_default()
}

fn optional_string_value(value: &DbValue) -> Option<String> {
    String::try_from(value.clone()).ok()
}

fn int_value(value: &DbValue) -> i64 {
    i64::try_from(value.clone()).unwrap_or(0)
}

fn quoted(identifier: &str) -> String {
    format!("'{}'", identifier.replace('\'', "''"))
}

fn parse_type_length(data_type: &str) -> Option<usize> {
    let open = data_type.find('(')?;
    let close = data_type[open..].find(')')? + open;
    data_type[open + 1..close].trim().parse().ok()
}

// ---------------------------------------------------------------------------
// Model fixtures
// ---------------------------------------------------------------------------

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
}

#[derive(Default)]
struct WidgetV2Config;

impl IEntityTypeConfiguration<WidgetV2> for WidgetV2Config {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, WidgetV2>) {
        entity.property_named("stock").has_default("0").has_index();
        entity.property_named("name").has_index();
    }
}

fn widget_v2_model() -> Vec<EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.apply_configuration::<WidgetV2Config, WidgetV2>();
    builder.build()
}

// ---------------------------------------------------------------------------
// Temp database
// ---------------------------------------------------------------------------

struct TempDb {
    path: PathBuf,
}

impl TempDb {
    fn new(tag: &str) -> TempDb {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!(
            "rust_ef_live_diff_{tag}_{}_{nanos}.db",
            std::process::id()
        ));
        TempDb { path }
    }

    fn provider(&self) -> Arc<SqliteProvider> {
        Arc::new(SqliteProvider::new(&self.path).expect("sqlite provider"))
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sqlite_upgrade_adds_column_and_index_preserving_data() {
    let database = TempDb::new("add_column");
    let provider = database.provider();
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Iteration 1: install the initial schema through the normal migration path.
    let initial = engine
        .generate("Initial", &[WidgetV1::entity_meta()], &None)
        .expect("generate initial");
    engine
        .apply(&*provider, &initial)
        .await
        .expect("apply initial");

    let mut connection = provider.get_connection().await.expect("connection");
    connection
        .execute(
            "INSERT INTO widgets (name) VALUES (?)",
            &[DbValue::String("old row".to_string())],
        )
        .await
        .expect("insert old row");

    // Iteration 2: the model grew a column + two indexes.
    let live = read_sqlite_schema(&*provider).await;
    let diff = engine.diff_metas(&widget_v2_model(), &live);

    let sql = diff.up_sql();
    // EF-12 infers NOT NULL for non-Option fields, so `stock: i32` is rendered
    // NOT NULL; the configured default backfills existing rows.
    assert!(
        sql.contains("ALTER TABLE \"widgets\" ADD COLUMN \"stock\" INTEGER NOT NULL DEFAULT 0"),
        "expected ADD COLUMN:\n{sql}"
    );
    assert!(
        sql.contains("CREATE INDEX \"ix_widgets_name\" ON \"widgets\" (\"name\")"),
        "expected index on name:\n{sql}"
    );
    assert!(
        sql.contains("CREATE INDEX \"ix_widgets_stock\" ON \"widgets\" (\"stock\")"),
        "expected index on stock:\n{sql}"
    );

    let applied = engine
        .apply_diff(&*provider, &diff)
        .await
        .expect("apply runtime diff");
    assert_eq!(applied.executed_statements, diff.statements.len());
    assert!(applied.rebuilt_tables.is_empty());

    // Existing rows are preserved and take the new column's default.
    let rows = connection
        .query("SELECT stock, name FROM widgets", &[])
        .await
        .expect("select rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(i64::try_from(rows[0][0].clone()).unwrap(), 0);
    assert_eq!(string_value(&rows[0][1]), "old row");

    // A second introspection produces an empty diff (idempotent).
    let live = read_sqlite_schema(&*provider).await;
    let second = engine.diff_metas(&widget_v2_model(), &live);
    assert!(
        second.is_empty(),
        "second diff must be empty:\n{}",
        second.up_sql()
    );
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);

    // Runtime diffs do not write migration history rows.
    let history = connection
        .query("SELECT COUNT(*) FROM __ef_migrations_history", &[])
        .await
        .expect("history count");
    assert_eq!(i64::try_from(history[0][0].clone()).unwrap(), 1);
}

#[tokio::test]
async fn apply_diff_rolls_back_every_statement_on_failure() {
    let database = TempDb::new("atomic");
    let provider = database.provider();
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let diff = MigrationDiff {
        statements: vec![
            "CREATE TABLE atomic_probe (id INTEGER PRIMARY KEY)".to_string(),
            "CREATE TABLE atomic_probe (id INTEGER PRIMARY KEY)".to_string(),
        ],
        warnings: Vec::new(),
        rebuilds: Vec::new(),
    };

    let error = engine
        .apply_diff(&*provider, &diff)
        .await
        .expect_err("duplicate CREATE TABLE must fail");
    let message = error.to_string();
    assert!(!message.is_empty());

    let live = read_sqlite_schema(&*provider).await;
    assert!(
        !live.iter().any(|table| table.name == "atomic_probe"),
        "failed plan must not leave a partially applied table: {live:?}"
    );
}

#[tokio::test]
async fn apply_diff_noop_is_reported() {
    let database = TempDb::new("noop");
    let provider = database.provider();
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let result = engine
        .apply_diff(&*provider, &MigrationDiff::default())
        .await
        .expect("noop apply");
    assert!(result.is_noop());
    assert_eq!(result.executed_statements, 0);
}

// ---------------------------------------------------------------------------
// Stage 3: SQLite table rebuild
// ---------------------------------------------------------------------------

use rust_ef::relations::DeleteBehavior;

#[derive(Debug, Clone, EntityType)]
#[table("ef_parents")]
struct ParentV1 {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("ef_children")]
struct ChildV1 {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub parent_id: i32,
    pub amount: i32,
}

#[derive(Debug, Clone, EntityType)]
#[table("ef_children")]
struct ChildV2 {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub parent_id: i32,
    pub amount: i32,
    pub note: Option<String>,
}

#[derive(Default)]
struct ChildV2Config;

impl IEntityTypeConfiguration<ChildV2> for ChildV2Config {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, ChildV2>) {
        entity
            .has_foreign_key(
                &[ChildV2::COLUMN_PARENT_ID],
                ParentV1::TABLE,
                &[ParentV1::COLUMN_ID],
            )
            .on_delete(DeleteBehavior::Cascade);
        entity.has_check("ck_children_amount", "amount >= 0");
        entity.property_named("amount").has_index();
        entity.property_named("note").has_default("'n/a'");
    }
}

#[derive(Debug, Clone, EntityType)]
#[table("ef_children")]
struct ChildV3 {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub parent_id: i32,
    pub amount: i32,
    pub code: String,
}

#[derive(Default)]
struct ChildV3Config;

impl IEntityTypeConfiguration<ChildV3> for ChildV3Config {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, ChildV3>) {
        entity.has_check("ck_children_code", "length(code) > 0");
        entity.property_named("code").is_required();
    }
}

fn child_v2_model() -> Vec<EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.entity::<ParentV1>();
    builder.apply_configuration::<ChildV2Config, ChildV2>();
    builder.build()
}

fn child_v3_model() -> Vec<EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.entity::<ParentV1>();
    builder.apply_configuration::<ChildV3Config, ChildV3>();
    builder.build()
}

async fn install_children_v1(provider: &dyn IDatabaseProvider) -> MigrationEngine {
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let initial = engine
        .generate(
            "Initial",
            &[ParentV1::entity_meta(), ChildV1::entity_meta()],
            &None,
        )
        .expect("generate initial");
    engine
        .apply(provider, &initial)
        .await
        .expect("apply initial");
    engine
}

async fn insert_child(
    provider: &dyn IDatabaseProvider,
    parent_id: i32,
    amount: i32,
) -> Box<dyn rust_ef::provider::IAsyncConnection> {
    let mut connection = provider.get_connection().await.expect("connection");
    connection
        .execute(
            "INSERT INTO ef_children (parent_id, amount) VALUES (?, ?)",
            &[DbValue::I32(parent_id), DbValue::I32(amount)],
        )
        .await
        .expect("insert child");
    connection
}

fn live_children(live: &[DbTable]) -> &DbTable {
    live.iter()
        .find(|table| table.name == "ef_children")
        .expect("ef_children in live schema")
}

#[tokio::test]
async fn sqlite_rebuild_applies_check_and_fk_preserving_data_and_triggers() {
    let database = TempDb::new("rebuild_ok");
    let provider = database.provider();
    let engine = install_children_v1(&*provider).await;

    let mut connection = provider.get_connection().await.expect("connection");
    connection
        .execute(
            "INSERT INTO ef_parents (name) VALUES (?)",
            &[DbValue::String("parent".to_string())],
        )
        .await
        .expect("insert parent");
    drop(insert_child(&*provider, 1, 5).await);
    drop(connection);

    provider
        .execute_migration_command(
            "CREATE TRIGGER trg_children_audit AFTER INSERT ON \"ef_children\" \
             BEGIN UPDATE \"ef_children\" SET amount = amount WHERE id = NEW.id; END",
        )
        .await
        .expect("create trigger");

    let live = read_sqlite_schema(&*provider).await;
    let diff = engine.diff_metas(&child_v2_model(), &live);
    assert_eq!(diff.rebuild_count(), 1, "diff:\n{}", diff.report());
    assert_eq!(diff.rebuilds[0].table, "ef_children");
    assert_eq!(diff.rebuilds[0].new_columns, vec!["note".to_string()]);
    assert!(diff.rebuilds[0].triggers_preserved);
    assert!(diff.rebuilds[0].not_null_columns_without_default.is_empty());

    let applied = engine
        .apply_diff(&*provider, &diff)
        .await
        .expect("apply rebuild");
    assert_eq!(applied.rebuilt_tables, vec!["ef_children".to_string()]);

    // Data preserved, new column got its default.
    let mut connection = provider.get_connection().await.expect("connection");
    let rows = connection
        .query("SELECT id, parent_id, amount, note FROM ef_children", &[])
        .await
        .expect("select children");
    assert_eq!(rows.len(), 1);
    assert_eq!(i64::try_from(rows[0][1].clone()).unwrap(), 1);
    assert_eq!(i64::try_from(rows[0][2].clone()).unwrap(), 5);
    assert_eq!(string_value(&rows[0][3]), "n/a");

    // Constraints, index and trigger are present after the rebuild.
    let live = read_sqlite_schema(&*provider).await;
    let children = live_children(&live);
    let checks = children.checks.as_ref().expect("checks introspected");
    assert!(
        checks
            .iter()
            .any(|check| check.expression.contains("amount >= 0")),
        "missing CHECK: {checks:?}"
    );
    assert!(
        children.foreign_keys.iter().any(|foreign_key| {
            foreign_key.columns == vec!["parent_id".to_string()]
                && foreign_key.referenced_table == "ef_parents"
                && foreign_key.referenced_columns == vec!["id".to_string()]
                && foreign_key.on_delete.as_deref() == Some("CASCADE")
        }),
        "missing FK: {:?}",
        children.foreign_keys
    );
    assert!(
        children
            .indexes
            .iter()
            .any(|index| index.name == "ix_ef_children_amount"),
        "missing index: {:?}",
        children.indexes
    );
    assert!(
        children
            .triggers
            .as_ref()
            .expect("triggers introspected")
            .iter()
            .any(|trigger| trigger.name == "trg_children_audit"),
        "missing trigger: {:?}",
        children.triggers
    );

    // CHECK is enforced.
    assert!(
        connection
            .execute(
                "INSERT INTO ef_children (parent_id, amount) VALUES (?, ?)",
                &[DbValue::I32(1), DbValue::I32(-1)],
            )
            .await
            .is_err(),
        "new CHECK constraint must reject negative amounts"
    );
    // The FK is enforced when foreign keys are enabled (EF-4 hook).
    connection
        .execute("PRAGMA foreign_keys = ON", &[])
        .await
        .expect("enable foreign keys");
    assert!(
        connection
            .execute(
                "INSERT INTO ef_children (parent_id, amount) VALUES (?, ?)",
                &[DbValue::I32(999), DbValue::I32(1)],
            )
            .await
            .is_err(),
        "new FK must reject missing parents"
    );

    // Idempotent: a second introspection matches the model.
    let live = read_sqlite_schema(&*provider).await;
    let second = engine.diff_metas(&child_v2_model(), &live);
    assert!(second.is_empty(), "second diff:\n{}", second.report());
}

#[tokio::test]
async fn sqlite_rebuild_rolls_back_when_existing_rows_violate_new_check() {
    let database = TempDb::new("rebuild_check_rollback");
    let provider = database.provider();
    let engine = install_children_v1(&*provider).await;

    provider
        .execute_migration_command("INSERT INTO ef_parents (id, name) VALUES (1, 'p')")
        .await
        .expect("insert parent");
    drop(insert_child(&*provider, 1, -5).await);

    let live = read_sqlite_schema(&*provider).await;
    let diff = engine.diff_metas(&child_v2_model(), &live);
    assert_eq!(diff.rebuild_count(), 1);

    let error = engine
        .apply_diff(&*provider, &diff)
        .await
        .expect_err("rows violating the new CHECK must abort the rebuild");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("check") || message.contains("constraint"),
        "unexpected error: {error}"
    );

    // Nothing was applied: no shadow table, no CHECK/FK, data intact.
    let mut connection = provider.get_connection().await.expect("connection");
    let leftovers = connection
        .query(
            "SELECT name FROM sqlite_master WHERE name LIKE '__ef_rebuild_%'",
            &[],
        )
        .await
        .expect("check leftovers");
    assert!(leftovers.is_empty(), "failed rebuild left {leftovers:?}");

    let live = read_sqlite_schema(&*provider).await;
    let children = live_children(&live);
    assert!(
        children.checks.as_ref().expect("checks").is_empty(),
        "CHECK must not be applied: {:?}",
        children.checks
    );
    assert!(children.foreign_keys.is_empty());
    let rows = connection
        .query("SELECT COUNT(*), MIN(amount) FROM ef_children", &[])
        .await
        .expect("select children");
    assert_eq!(i64::try_from(rows[0][0].clone()).unwrap(), 1);
    assert_eq!(i64::try_from(rows[0][1].clone()).unwrap(), -5);
}

#[tokio::test]
async fn sqlite_rebuild_rolls_back_when_foreign_key_check_finds_orphans() {
    let database = TempDb::new("rebuild_fk_rollback");
    let provider = database.provider();
    let engine = install_children_v1(&*provider).await;

    // Orphan child: `parent_id = 999` does not exist (v1 has no FK yet).
    drop(insert_child(&*provider, 999, 1).await);

    let live = read_sqlite_schema(&*provider).await;
    let diff = engine.diff_metas(&child_v2_model(), &live);
    assert_eq!(diff.rebuild_count(), 1);

    let error = engine
        .apply_diff(&*provider, &diff)
        .await
        .expect_err("orphan rows must fail the foreign-key check");
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("foreign-key") || message.contains("foreign key"),
        "unexpected error: {error}"
    );

    // Rolled back: the FK is not applied and the orphan row is untouched.
    let live = read_sqlite_schema(&*provider).await;
    let children = live_children(&live);
    assert!(children.foreign_keys.is_empty(), "FK must not be applied");
    let mut connection = provider.get_connection().await.expect("connection");
    let rows = connection
        .query("SELECT COUNT(*) FROM ef_children", &[])
        .await
        .expect("select children");
    assert_eq!(i64::try_from(rows[0][0].clone()).unwrap(), 1);
}

#[tokio::test]
async fn apply_diff_rejects_rebuild_that_would_need_not_null_backfill() {
    let database = TempDb::new("rebuild_not_null");
    let provider = database.provider();
    let engine = install_children_v1(&*provider).await;
    drop(insert_child(&*provider, 1, 3).await);

    let live = read_sqlite_schema(&*provider).await;
    let diff = engine.diff_metas(&child_v3_model(), &live);
    assert_eq!(diff.rebuild_count(), 1);
    assert_eq!(
        diff.rebuilds[0].not_null_columns_without_default,
        vec!["code".to_string()]
    );

    let error = engine
        .apply_diff(&*provider, &diff)
        .await
        .expect_err("NOT NULL column without default must be rejected up-front");
    assert!(
        error.to_string().contains("NOT NULL"),
        "unexpected error: {error}"
    );

    // Rejected before touching the database.
    let live = read_sqlite_schema(&*provider).await;
    let children = live_children(&live);
    assert!(
        !children.columns.iter().any(|column| column.name == "code"),
        "column must not be added"
    );
    assert!(children.checks.as_ref().expect("checks").is_empty());
}

#[tokio::test]
async fn runtime_diff_migration_revert_returns_typed_error() {
    let database = TempDb::new("into_migration_revert");
    let provider = database.provider();
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    let initial = engine
        .generate("Initial", &[WidgetV1::entity_meta()], &None)
        .expect("generate initial");
    engine
        .apply(&*provider, &initial)
        .await
        .expect("apply initial");

    let live = read_sqlite_schema(&*provider).await;
    let diff = engine.diff_metas(&widget_v2_model(), &live);
    assert!(
        !diff.is_empty(),
        "expected an upgrade plan:\n{}",
        diff.report()
    );
    let migration = diff.into_migration("Runtime_Upgrade", MigrationDialect::Sqlite);
    assert!(
        migration
            .down_sql
            .starts_with("-- rust-ef runtime diff: no automatic down migration"),
        "the down script must carry the no-automatic-down marker: {}",
        migration.down_sql
    );

    engine
        .apply(&*provider, &migration)
        .await
        .expect("apply runtime migration");
    assert!(engine
        .is_applied(&*provider, "Runtime_Upgrade")
        .await
        .expect("history"));

    // A runtime upgrade has no safe inverse: revert must fail loudly instead of
    // deleting the history row and pretending the schema was rolled back.
    let error = engine
        .revert(&*provider, &migration)
        .await
        .expect_err("runtime upgrades cannot be reverted automatically");
    assert!(
        error.to_string().contains("no automatic down"),
        "unexpected error: {error}"
    );

    // State stays consistent: the history row is still there and the schema is
    // still upgraded (no half-reverted database).
    assert!(engine
        .is_applied(&*provider, "Runtime_Upgrade")
        .await
        .expect("history"));
    let mut connection = provider.get_connection().await.expect("connection");
    let columns = connection
        .query("PRAGMA table_info('widgets')", &[])
        .await
        .expect("table_info");
    assert!(
        columns.iter().any(|row| string_value(&row[1]) == "stock"),
        "the applied upgrade must stay in place"
    );

    // The history guard keeps re-applying idempotent.
    engine
        .apply(&*provider, &migration)
        .await
        .expect("apply is idempotent while recorded as applied");
}

// ---------------------------------------------------------------------------
// EF-9 addendum: reverse of a purely additive runtime diff
// ---------------------------------------------------------------------------

/// Same table/model as `WidgetV2` but with a different default for `stock`, so
/// SQLite has to rebuild the table (a non-additive change).
#[derive(Debug, Clone, EntityType)]
#[table("widgets")]
struct WidgetV3 {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub name: String,
    pub stock: i32,
}

#[derive(Default)]
struct WidgetV3Config;

impl IEntityTypeConfiguration<WidgetV3> for WidgetV3Config {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, WidgetV3>) {
        entity.property_named("stock").has_default("7").has_index();
        entity.property_named("name").has_index();
    }
}

fn widget_v3_model() -> Vec<EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.apply_configuration::<WidgetV3Config, WidgetV3>();
    builder.build()
}

#[tokio::test]
async fn additive_runtime_diff_reverse_removes_created_objects_end_to_end() {
    let database = TempDb::new("reverse_additive");
    let provider = database.provider();
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Fresh database: the whole plan is additive (tables + indexes).
    let live = read_sqlite_schema(&*provider).await;
    assert!(live.is_empty(), "expected a fresh database: {live:?}");
    let diff = engine.diff_metas(&widget_v2_model(), &live);
    assert!(!diff.is_empty());
    assert!(
        diff.irreversible_items().is_empty(),
        "purely additive plan must be reversible: {:?}",
        diff.irreversible_items()
    );
    assert!(
        diff.report().contains("irreversible items: 0"),
        "report must list irreversible items:\n{}",
        diff.report()
    );

    let applied = engine
        .apply_diff(&*provider, &diff)
        .await
        .expect("apply forward plan");
    assert_eq!(applied.executed_statements, diff.statements.len());
    let live = read_sqlite_schema(&*provider).await;
    assert!(
        live.iter().any(|table| table.name == "widgets"),
        "forward plan must create the table: {live:?}"
    );

    let reverse = diff
        .reverse(MigrationDialect::Sqlite)
        .expect("a purely additive diff is reversible");
    assert!(!reverse.statements.is_empty());
    engine
        .apply_diff(&*provider, &reverse)
        .await
        .expect("apply reverse plan");

    // Everything the forward plan created is gone again.
    let live = read_sqlite_schema(&*provider).await;
    assert!(
        live.is_empty(),
        "reverse plan must remove the created tables/indexes: {live:?}"
    );
}

#[tokio::test]
async fn runtime_diff_with_table_rebuild_is_not_reversible() {
    let database = TempDb::new("reverse_rebuild");
    let provider = database.provider();
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);

    // Install v1, upgrade to v2 additively.
    let initial = engine
        .generate("Initial", &[WidgetV1::entity_meta()], &None)
        .expect("generate initial");
    engine
        .apply(&*provider, &initial)
        .await
        .expect("apply initial");
    let live = read_sqlite_schema(&*provider).await;
    let v2 = engine.diff_metas(&widget_v2_model(), &live);
    engine
        .apply_diff(&*provider, &v2)
        .await
        .expect("apply additive upgrade");

    // Changing the `stock` default makes SQLite rebuild the table.
    let live = read_sqlite_schema(&*provider).await;
    let diff = engine.diff_metas(&widget_v3_model(), &live);
    assert_eq!(
        diff.rebuild_count(),
        1,
        "expected a table rebuild:\n{}",
        diff.report()
    );
    let items = diff.irreversible_items();
    assert!(
        items
            .iter()
            .any(|item| item.contains("table rebuild required for widgets")),
        "items: {items:?}"
    );
    // The rebuild's own shadow-table statements are reported as well; they are
    // equally non-invertible.
    assert!(
        items
            .iter()
            .any(|item| item.starts_with("destructive/unsupported statement: DROP TABLE")),
        "items: {items:?}"
    );
    let report = diff.report();
    assert!(
        report.contains("irreversible items: ") && !report.contains("irreversible items: 0"),
        "report:\n{report}"
    );

    let error = diff
        .reverse(MigrationDialect::Sqlite)
        .expect_err("a plan with a table rebuild has no automatic inverse");
    assert!(
        error.to_string().contains("not reversible"),
        "unexpected error: {error}"
    );
}
