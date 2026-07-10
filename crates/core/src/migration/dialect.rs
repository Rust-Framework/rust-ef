//! SQL dialect enumeration and type mapping for migration DDL generation.

use super::types::SnapshotColumn;

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

        // Sequence handling (PostgreSQL: plain type + DEFAULT nextval in DDL;
        // non-PG: fall back to auto_increment syntax)
        if col.is_sequence {
            if tn.ends_with("i32") {
                return match self {
                    MigrationDialect::Postgres => "INTEGER".into(),
                    MigrationDialect::MySql => "INT AUTO_INCREMENT".into(),
                    MigrationDialect::Sqlite => "INTEGER".into(),
                };
            }
            if tn.ends_with("i64") {
                return match self {
                    MigrationDialect::Postgres => "BIGINT".into(),
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
