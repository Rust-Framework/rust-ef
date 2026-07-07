//! Public data types for the migration system.
//!
//! `Migration` / `ModelSnapshot` / `SnapshotEntityType` / `SnapshotColumn`
//! are serialized forms used by the engine and store. `MigrationDialect`
//! encapsulates SQL dialect differences. `SchemaChange` is the internal
//! diff result enum consumed by the SQL generator.

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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnapshotColumn {
    pub field_name: String,
    pub column_name: String,
    pub type_name: String,
    pub is_primary_key: bool,
    pub is_required: bool,
    pub is_foreign_key: bool,
    pub max_length: Option<usize>,
    pub is_auto_increment: bool,
    /// Referenced table when `is_foreign_key` is true.
    pub fk_referenced_table: Option<String>,
    /// Referenced column when `is_foreign_key` is true.
    pub fk_referenced_column: Option<String>,
    /// Non-unique index on this column.
    pub has_index: bool,
    /// Unique constraint/index on this column.
    pub is_unique: bool,
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
        // type_name comes from std::any::type_name::<T>() which returns
        // fully-qualified paths (e.g. "alloc::string::String"). Use ends_with
        // / contains matching to handle both simple and qualified names.
        let tn = col.type_name.as_str();

        // Auto-increment handling (must be checked before plain i32/i64)
        if col.is_auto_increment {
            if tn.ends_with("i32") {
                return match self {
                    MigrationDialect::Postgres => "SERIAL".into(),
                    MigrationDialect::MySql => "INT AUTO_INCREMENT".into(),
                    MigrationDialect::Sqlite => "INTEGER".into(),
                };
            }
            if tn.ends_with("i64") {
                return match self {
                    MigrationDialect::Postgres => "BIGSERIAL".into(),
                    MigrationDialect::MySql => "BIGINT AUTO_INCREMENT".into(),
                    MigrationDialect::Sqlite => "INTEGER".into(),
                };
            }
        }

        let base: &str = if tn.ends_with("i16") {
            "SMALLINT"
        } else if tn.ends_with("i32") {
            "INTEGER"
        } else if tn.ends_with("i64") {
            "BIGINT"
        } else if tn.ends_with("f32") {
            "REAL"
        } else if tn.ends_with("f64") {
            "DOUBLE PRECISION"
        } else if tn.ends_with("bool") {
            "BOOLEAN"
        } else if tn.ends_with("String") {
            return match col.max_length {
                Some(n) => format!("VARCHAR({})", n),
                None => "TEXT".into(),
            };
        } else if tn.ends_with("Vec<u8>") {
            return match self {
                MigrationDialect::Postgres => "BYTEA".into(),
                MigrationDialect::MySql | MigrationDialect::Sqlite => "BLOB".into(),
            };
        } else if tn.contains("NaiveDateTime") {
            return match self {
                MigrationDialect::Postgres => "TIMESTAMP".into(),
                MigrationDialect::MySql => "DATETIME".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("NaiveDate") {
            return match self {
                MigrationDialect::Postgres => "DATE".into(),
                MigrationDialect::MySql => "DATE".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("DateTime") {
            // chrono::DateTime<Utc> → TIMESTAMPTZ (PG) / DATETIME (MySQL) / TEXT (SQLite)
            return match self {
                MigrationDialect::Postgres => "TIMESTAMPTZ".into(),
                MigrationDialect::MySql => "DATETIME".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("Uuid") {
            return match self {
                MigrationDialect::Postgres => "UUID".into(),
                MigrationDialect::MySql => "CHAR(36)".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("Decimal") {
            return match self {
                MigrationDialect::Postgres => "NUMERIC".into(),
                MigrationDialect::MySql => "DECIMAL(38,18)".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else {
            "TEXT"
        };
        base.to_string()
    }
}

/// Single atomic schema change detected during diffing.
#[derive(Debug, Clone)]
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
        referenced_table: String,
    },
    CreateIndex {
        table: String,
        column: String,
        is_unique: bool,
    },
    DropIndex {
        table: String,
        column: String,
        is_unique: bool,
    },
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

/// Product version recorded in migration history.
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");
