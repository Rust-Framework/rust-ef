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
use rust_ef::migration::{
    MigrationDiff, MigrationDialect, MigrationEngine,
};
use rust_ef::model_builder::{EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder};
use rust_ef::provider::{DbValue, IDatabaseProvider};
use rust_ef::schema::{DbCheck, DbColumn, DbForeignKey, DbIndex, DbIndexColumn, DbTable, DbTrigger};
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
            let referenced_columns: Vec<String> =
                parts.iter().map(|part| part.3.clone().unwrap_or_default()).collect();
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
        let open = create_sql[position..].find('(').map(|index| position + index);
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
    engine.apply(&*provider, &initial).await.expect("apply initial");

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
    assert!(
        sql.contains("ALTER TABLE \"widgets\" ADD COLUMN \"stock\" INTEGER NULL DEFAULT 0"),
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
