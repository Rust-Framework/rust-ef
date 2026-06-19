//! Migration engine — model snapshot diffing, migration generation, and history tracking.
//!
//! Corresponds to EFCore's migration system. Implements full model diffing:
//!   - Detect added/removed tables
//!   - Detect added/removed/altered columns
//!   - Generate Up/Down SQL with dialect-specific type mappings
//!   - Maintain `__ef_migrations_history` tracking table

use crate::error::LrefResult;
use crate::metadata::EntityTypeMeta;
use std::collections::{HashMap, HashSet};

/// Represents a single migration with up/down SQL scripts.
#[derive(Debug, Clone)]
pub struct Migration {
    pub id: String,
    pub description: String,
    pub up_sql: String,
    pub down_sql: String,
}

/// A snapshot of the entity model at a point in time.
/// Corresponds to EFCore's `ModelSnapshot`.
#[derive(Debug, Clone)]
pub struct ModelSnapshot {
    pub migration_id: String,
    pub entity_types: Vec<SnapshotEntityType>,
}

/// Serialized form of EntityTypeMeta for snapshot storage.
#[derive(Debug, Clone)]
pub struct SnapshotEntityType {
    pub type_name: String,
    pub table_name: String,
    pub columns: Vec<SnapshotColumn>,
}

/// Serialized form of PropertyMeta for snapshot storage.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotColumn {
    pub field_name: String,
    pub column_name: String,
    pub type_name: String,
    pub is_primary_key: bool,
    pub is_required: bool,
    pub is_foreign_key: bool,
    pub max_length: Option<usize>,
    pub is_auto_increment: bool,
}

/// Specifies the database SQL dialect for migration generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDialect {
    Postgres,
    MySql,
    Sqlite,
}

impl MigrationDialect {
    /// Quote an identifier according to dialect rules.
    pub fn quote(&self, ident: &str) -> String {
        match self {
            MigrationDialect::Postgres | MigrationDialect::Sqlite => format!("\"{}\"", ident),
            MigrationDialect::MySql => format!("`{}`", ident),
        }
    }

    /// Map a Rust type name to the dialect-specific column type.
    pub fn map_column_type(&self, col: &SnapshotColumn) -> String {
        let base = match (col.type_name.as_str(), col.max_length) {
            ("i32", _) if col.is_auto_increment => {
                match self {
                    MigrationDialect::Postgres => "SERIAL",
                    MigrationDialect::MySql => "INT AUTO_INCREMENT",
                    MigrationDialect::Sqlite => "INTEGER",
                }
            }
            ("i64", _) if col.is_auto_increment => {
                match self {
                    MigrationDialect::Postgres => "BIGSERIAL",
                    MigrationDialect::MySql => "BIGINT AUTO_INCREMENT",
                    MigrationDialect::Sqlite => "INTEGER",
                }
            }
            ("i16", _) => "SMALLINT",
            ("i32", _) => "INTEGER",
            ("i64", _) => "BIGINT",
            ("f32", _) => "REAL",
            ("f64", _) => "DOUBLE PRECISION",
            ("bool", _) => "BOOLEAN",
            ("String", Some(n)) => return format!("VARCHAR({})", n),
            ("String", None) => "TEXT",
            ("Vec<u8>", _) => "BYTEA",
            _ => "TEXT",
        };
        base.to_string()
    }
}

/// Single atomic schema change detected during diffing.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum SchemaChange {
    CreateTable {
        table: String,
        columns: Vec<SnapshotColumn>,
    },
    DropTable {
        table: String,
    },
    AddColumn {
        table: String,
        column: SnapshotColumn,
    },
    DropColumn {
        table: String,
        column_name: String,
    },
    AlterColumn {
        table: String,
        column_name: String,
        old: SnapshotColumn,
        new: SnapshotColumn,
    },
    AddForeignKey {
        table: String,
        column: String,
        referenced_table: String,
        referenced_column: String,
    },
    DropForeignKey {
        table: String,
        column: String,
    },
}

/// The migration engine — compares old and new model snapshots to generate
/// migration SQL.
pub struct MigrationEngine {
    dialect: MigrationDialect,
}

impl MigrationEngine {
    pub fn new(dialect: MigrationDialect) -> Self {
        Self { dialect }
    }

    /// Generates a migration by diffing the current model against a snapshot.
    pub fn generate(
        &self,
        name: &str,
        current: &[EntityTypeMeta],
        previous_snapshot: &Option<ModelSnapshot>,
    ) -> LrefResult<Migration> {
        let current_snapshot = self.create_snapshot("__current__", current);

        let changes = match previous_snapshot {
            Some(prev) => self.diff(prev, &current_snapshot),
            None => {
                // First migration: create all tables
                self.initial_create(&current_snapshot)
            }
        };

        let up_sql = self.generate_up_sql(&changes);
        let down_sql = self.generate_down_sql(&changes);

        Ok(Migration {
            id: name.to_string(),
            description: name.to_string(),
            up_sql,
            down_sql,
        })
    }

    /// Creates a snapshot from current entity type metadata.
    pub fn create_snapshot(
        &self,
        migration_id: &str,
        entity_types: &[EntityTypeMeta],
    ) -> ModelSnapshot {
        let types = entity_types
            .iter()
            .map(|et| SnapshotEntityType {
                type_name: et.type_name.to_string(),
                table_name: et.table_name.to_string(),
                columns: et
                    .properties
                    .iter()
                    .filter(|p| !p.is_not_mapped)
                    .map(|p| SnapshotColumn {
                        field_name: p.field_name.to_string(),
                        column_name: p.column_name.to_string(),
                        type_name: p.type_name.to_string(),
                        is_primary_key: p.is_primary_key,
                        is_required: p.is_required,
                        is_foreign_key: p.is_foreign_key,
                        max_length: p.max_length,
                        is_auto_increment: p.is_auto_increment,
                    })
                    .collect(),
            })
            .collect();

        ModelSnapshot {
            migration_id: migration_id.to_string(),
            entity_types: types,
        }
    }

    // -----------------------------------------------------------------------
    // Diffing
    // -----------------------------------------------------------------------

    fn initial_create(&self, current: &ModelSnapshot) -> Vec<SchemaChange> {
        current
            .entity_types
            .iter()
            .map(|et| SchemaChange::CreateTable {
                table: et.table_name.clone(),
                columns: et.columns.clone(),
            })
            .collect()
    }

    fn diff(&self, old: &ModelSnapshot, new: &ModelSnapshot) -> Vec<SchemaChange> {
        let mut changes = Vec::new();

        // Build lookup maps by table name
        let old_tables: HashMap<&str, &SnapshotEntityType> = old
            .entity_types
            .iter()
            .map(|e| (e.table_name.as_str(), e))
            .collect();
        let new_tables: HashMap<&str, &SnapshotEntityType> = new
            .entity_types
            .iter()
            .map(|e| (e.table_name.as_str(), e))
            .collect();

        let old_names: HashSet<&str> = old_tables.keys().copied().collect();
        let new_names: HashSet<&str> = new_tables.keys().copied().collect();

        // Dropped tables
        for name in old_names.difference(&new_names) {
            changes.push(SchemaChange::DropTable {
                table: name.to_string(),
            });
        }

        // New tables
        for name in new_names.difference(&old_names) {
            let et = new_tables[name];
            changes.push(SchemaChange::CreateTable {
                table: et.table_name.clone(),
                columns: et.columns.clone(),
            });
        }

        // Tables present in both — compare columns
        for name in old_names.intersection(&new_names) {
            let old_et = old_tables[name];
            let new_et = new_tables[name];
            let table = &old_et.table_name;

            let old_cols: HashMap<&str, &SnapshotColumn> = old_et
                .columns
                .iter()
                .map(|c| (c.column_name.as_str(), c))
                .collect();
            let new_cols: HashMap<&str, &SnapshotColumn> = new_et
                .columns
                .iter()
                .map(|c| (c.column_name.as_str(), c))
                .collect();

            let old_col_names: HashSet<&str> = old_cols.keys().copied().collect();
            let new_col_names: HashSet<&str> = new_cols.keys().copied().collect();

            // Added columns
            for col_name in new_col_names.difference(&old_col_names) {
                changes.push(SchemaChange::AddColumn {
                    table: table.clone(),
                    column: new_cols[col_name].clone(),
                });
            }

            // Dropped columns
            for col_name in old_col_names.difference(&new_col_names) {
                changes.push(SchemaChange::DropColumn {
                    table: table.clone(),
                    column_name: col_name.to_string(),
                });
            }

            // Altered columns (present in both, but attributes differ)
            for col_name in old_col_names.intersection(&new_col_names) {
                let old_col = old_cols[col_name];
                let new_col = new_cols[col_name];

                if old_col != new_col {
                    changes.push(SchemaChange::AlterColumn {
                        table: table.clone(),
                        column_name: col_name.to_string(),
                        old: (*old_col).clone(),
                        new: (*new_col).clone(),
                    });
                }
            }
        }

        changes
    }

    // -----------------------------------------------------------------------
    // SQL generation
    // -----------------------------------------------------------------------

    fn generate_up_sql(&self, changes: &[SchemaChange]) -> String {
        let mut sql = String::from("-- Up Migration (auto-generated by rust-ef)\n\n");
        let q = |s: &str| self.dialect.quote(s);

        for change in changes {
            match change {
                SchemaChange::CreateTable { table, columns } => {
                    sql.push_str(&format!("CREATE TABLE {} (\n", q(table)));

                    // Separate primary key columns from regular columns
                    let pk_columns: Vec<&str> = columns
                        .iter()
                        .filter(|c| c.is_primary_key)
                        .map(|c| c.column_name.as_str())
                        .collect();

                    let col_defs: Vec<String> = columns
                        .iter()
                        .map(|c| {
                            let nullable = if c.is_required { "NOT NULL" } else { "NULL" };
                            let col_type = self.dialect.map_column_type(c);
                            // Don't put PRIMARY KEY on individual columns; handle separately
                            [q(&c.column_name), col_type, nullable.to_string()]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .collect();
                    sql.push_str(&format!("    {}\n", col_defs.join(",\n    ")));

                    // Composite primary key constraint
                    if pk_columns.len() == 1 {
                        sql.push_str(&format!("    ,PRIMARY KEY ({})\n", q(pk_columns[0])));
                    } else if pk_columns.len() > 1 {
                        let pk_list: Vec<String> = pk_columns.iter().map(|c| q(c)).collect();
                        sql.push_str(&format!("    ,PRIMARY KEY ({})\n", pk_list.join(", ")));
                    }

                    sql.push_str(");\n\n");
                }
                SchemaChange::DropTable { table } => {
                    sql.push_str(&format!("DROP TABLE IF EXISTS {};\n", q(table)));
                }
                SchemaChange::AddColumn { table, column } => {
                    let col_type = self.dialect.map_column_type(column);
                    let nullable = if column.is_required { "NOT NULL" } else { "NULL" };
                    sql.push_str(&format!(
                        "ALTER TABLE {} ADD COLUMN {} {} {};\n",
                        q(table),
                        q(&column.column_name),
                        col_type,
                        nullable
                    ));
                }
                SchemaChange::DropColumn { table, column_name } => {
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP COLUMN IF EXISTS {};\n",
                        q(table),
                        q(column_name)
                    ));
                }
                SchemaChange::AlterColumn {
                    table,
                    column_name,
                    old: _,
                    new,
                } => {
                    let col_type = self.dialect.map_column_type(new);
                    let nullable = if new.is_required { "SET NOT NULL" } else { "DROP NOT NULL" };
                    sql.push_str(&format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {};\n",
                        q(table),
                        q(column_name),
                        col_type
                    ));
                    sql.push_str(&format!(
                        "ALTER TABLE {} ALTER COLUMN {} {};\n",
                        q(table),
                        q(column_name),
                        nullable
                    ));
                }
                SchemaChange::AddForeignKey {
                    table,
                    column,
                    referenced_table,
                    referenced_column,
                } => {
                    let fk_name = format!("fk_{}_{}_{}", table, column, referenced_table);
                    sql.push_str(&format!(
                        "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({});\n",
                        q(table),
                        q(&fk_name),
                        q(column),
                        q(referenced_table),
                        q(referenced_column)
                    ));
                }
                SchemaChange::DropForeignKey { table, column } => {
                    // Standard naming convention matching AddForeignKey
                    // e.g., fk_table_column_referencedtable — simple match by column
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                        q(table),
                        q(&format!("fk_{}_{}", table, column))
                    ));
                }
            }
        }

        // Append migration history record
        sql.push_str(&format!(
            "INSERT INTO {}(migration_id, product_version) VALUES ('{{migration_id}}', '0.1.0');\n",
            q(MIGRATION_HISTORY_TABLE)
        ));

        sql
    }

    fn generate_down_sql(&self, changes: &[SchemaChange]) -> String {
        let mut sql = String::from("-- Down Migration (auto-generated by rust-ef)\n\n");
        let q = |s: &str| self.dialect.quote(s);

        // Reverse the changes for the down migration
        for change in changes.iter().rev() {
            match change {
                SchemaChange::CreateTable { table, .. } => {
                    sql.push_str(&format!("DROP TABLE IF EXISTS {};\n", q(table)));
                }
                SchemaChange::DropTable { table } => {
                    // Can't fully restore a dropped table — log warning
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore table {} (original schema unknown)\n",
                        q(table)
                    ));
                }
                SchemaChange::AddColumn { table, column } => {
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP COLUMN IF EXISTS {};\n",
                        q(table),
                        q(&column.column_name)
                    ));
                }
                SchemaChange::DropColumn { table, column_name } => {
                    // Can't fully restore dropped column without original type
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore column {} on {} (original type unknown)\n",
                        q(column_name),
                        q(table)
                    ));
                }
                SchemaChange::AlterColumn {
                    table,
                    column_name,
                    old,
                    new: _,
                } => {
                    let col_type = self.dialect.map_column_type(old);
                    sql.push_str(&format!(
                        "ALTER TABLE {} ALTER COLUMN {} TYPE {};\n",
                        q(table),
                        q(column_name),
                        col_type
                    ));
                }
                SchemaChange::AddForeignKey { table, column, .. } => {
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                        q(table),
                        q(&format!("fk_{}", column))
                    ));
                }
                SchemaChange::DropForeignKey {
                    table,
                    column,
                    ..
                } => {
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore foreign key constraint on {}.{}\n",
                        q(table),
                        q(column)
                    ));
                }
            }
        }

        sql.push_str(&format!(
            "DELETE FROM {} WHERE migration_id = '{{migration_id}}';\n",
            q(MIGRATION_HISTORY_TABLE)
        ));

        sql
    }
}

/// Represents a record in the migration history table.
/// Table name: `__ef_migrations_history`
/// Corresponds to EFCore's `__EFMigrationsHistory`.
#[derive(Debug, Clone)]
pub struct MigrationHistoryEntry {
    pub migration_id: String,
    pub product_version: String,
}

/// Table name for migration history.
pub const MIGRATION_HISTORY_TABLE: &str = "__ef_migrations_history";

/// SQL to create the migration history table.
pub fn create_migration_history_table_sql(dialect: MigrationDialect) -> String {
    match dialect {
        MigrationDialect::Postgres => format!(
            r#"CREATE TABLE IF NOT EXISTS "{table}" (
    "migration_id" VARCHAR(150) NOT NULL PRIMARY KEY,
    "product_version" VARCHAR(32) NOT NULL,
    "applied_at" TIMESTAMPTZ NOT NULL DEFAULT NOW()
);"#,
            table = MIGRATION_HISTORY_TABLE
        ),
        MigrationDialect::MySql => format!(
            r#"CREATE TABLE IF NOT EXISTS `{table}` (
    `migration_id` VARCHAR(150) NOT NULL PRIMARY KEY,
    `product_version` VARCHAR(32) NOT NULL,
    `applied_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"#,
            table = MIGRATION_HISTORY_TABLE
        ),
        MigrationDialect::Sqlite => format!(
            r#"CREATE TABLE IF NOT EXISTS "{table}" (
    "migration_id" TEXT NOT NULL PRIMARY KEY,
    "product_version" TEXT NOT NULL
);"#,
            table = MIGRATION_HISTORY_TABLE
        ),
    }
}
