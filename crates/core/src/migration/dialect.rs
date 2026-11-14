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
    ///
    /// Dotted identifiers are treated as `schema.table` and each part is quoted
    /// separately (e.g. `"audit"."logs"` / `` `audit`.`logs` ``).
    pub fn quote(&self, ident: &str) -> String {
        match self {
            MigrationDialect::Postgres | MigrationDialect::Sqlite => quote_qualified(ident, '"'),
            MigrationDialect::MySql => quote_qualified(ident, '`'),
        }
    }

    /// Map a Rust type name to the dialect-specific column type.
    pub fn map_column_type(&self, col: &SnapshotColumn) -> String {
        // type_name comes from std::any::type_name::<T>() which returns
        // fully-qualified paths (e.g. "alloc::string::String"). Use ends_with
        // / contains matching to handle both simple and qualified names.
        //
        // Explicit `HasColumnType` wins verbatim (user is responsible for dialect
        // portability), except for the one combination that cannot work on
        // MySQL: an unbounded `TEXT` column with a declared `max_length`.
        // MySQL cannot index an unbounded TEXT/BLOB, so a custom enum mapped to
        // TEXT would make `CREATE INDEX` fail with error 1170 even though
        // `max_length` was declared (EF-MYSQL-01). The declared length wins.
        if let Some(explicit) = &col.column_type {
            let explicit = explicit.trim();
            if !explicit.is_empty() {
                if matches!(self, MigrationDialect::MySql)
                    && explicit.eq_ignore_ascii_case("TEXT")
                    && col.max_length.is_some()
                {
                    return mysql_text_type(col);
                }
                return explicit.to_string();
            }
        }

        // `Option<T>` is unwrapped to its inner type so nullable columns keep
        // their inner mapping (e.g. `Option<String>` + max_length → VARCHAR(n)
        // instead of TEXT, which is not indexable on MySQL).
        let tn = inner_type_name(&col.type_name);

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
            if col.is_fixed_length {
                // Fixed-length string (`CHAR(n)`); a length is required by
                // MySQL. `max_length` normally supplies it.
                return format!("CHAR({})", col.max_length.unwrap_or(1));
            }
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
            return match (self, col.precision) {
                (MigrationDialect::Postgres, Some(p)) => format!("TIMESTAMP({p})"),
                (MigrationDialect::MySql, Some(p)) => format!("DATETIME({p})"),
                (MigrationDialect::Postgres, None) => "TIMESTAMP".into(),
                (MigrationDialect::MySql, None) => "DATETIME".into(),
                (MigrationDialect::Sqlite, _) => "TEXT".into(),
            };
        } else if tn.contains("NaiveDate") {
            return match self {
                MigrationDialect::Postgres => "DATE".into(),
                MigrationDialect::MySql => "DATE".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("DateTime") {
            // chrono::DateTime<Utc> → TIMESTAMPTZ (PG) / DATETIME (MySQL) / TEXT (SQLite)
            return match (self, col.precision) {
                (MigrationDialect::Postgres, Some(p)) => format!("TIMESTAMPTZ({p})"),
                (MigrationDialect::MySql, Some(p)) => format!("DATETIME({p})"),
                (MigrationDialect::Postgres, None) => "TIMESTAMPTZ".into(),
                (MigrationDialect::MySql, None) => "DATETIME".into(),
                (MigrationDialect::Sqlite, _) => "TEXT".into(),
            };
        } else if tn.contains("Uuid") {
            return match self {
                MigrationDialect::Postgres => "UUID".into(),
                MigrationDialect::MySql => "CHAR(36)".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("Decimal") {
            let (p, s) = (col.precision.unwrap_or(38), col.scale.unwrap_or(18));
            return match self {
                MigrationDialect::Postgres => {
                    if col.precision.is_some() {
                        format!("NUMERIC({p},{s})")
                    } else {
                        "NUMERIC".into()
                    }
                }
                MigrationDialect::MySql => format!("DECIMAL({p},{s})"),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if matches!(self, MigrationDialect::MySql) {
            // Custom/scalar types (contract enums, newtypes, ...) have no native
            // mapping. MySQL's default TEXT is not indexable; a declared
            // max_length therefore selects VARCHAR(n) (or CHAR(n) for
            // fixed-length columns). PostgreSQL/SQLite keep TEXT, which is both
            // indexable and the type existing databases were created with.
            return mysql_text_type(col);
        } else {
            "TEXT"
        };
        base.to_string()
    }
}

/// `VARCHAR(n)` / `CHAR(n)` for a MySQL text-like column with a declared
/// length; unbounded `TEXT` when no length was configured.
fn mysql_text_type(col: &SnapshotColumn) -> String {
    match col.max_length {
        Some(length) if col.is_fixed_length => format!("CHAR({length})"),
        Some(length) => format!("VARCHAR({length})"),
        None => "TEXT".to_string(),
    }
}

/// Quotes a possibly dotted identifier (`schema.table`) part by part.
pub(crate) fn quote_qualified(ident: &str, quote: char) -> String {
    ident
        .split('.')
        .map(|part| format!("{quote}{part}{quote}"))
        .collect::<Vec<_>>()
        .join(".")
}

/// Returns the inner type name for `Option<T>`, or the input unchanged.
///
/// Handles both `Option<T>` and fully-qualified
/// `core::option::Option<T>` / `std::option::Option<T>` spellings produced by
/// `std::any::type_name`.
fn inner_type_name(type_name: &str) -> &str {
    if let Some(pos) = type_name.find("Option<") {
        let rest = &type_name[pos + "Option<".len()..];
        return rest.strip_suffix('>').unwrap_or(rest);
    }
    type_name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_string_uses_inner_type_and_max_length() {
        let col = SnapshotColumn {
            type_name: "core::option::Option<alloc::string::String>".into(),
            max_length: Some(32),
            ..Default::default()
        };
        assert_eq!(MigrationDialect::MySql.map_column_type(&col), "VARCHAR(32)");
        assert_eq!(
            MigrationDialect::Postgres.map_column_type(&col),
            "VARCHAR(32)"
        );
    }

    #[test]
    fn custom_enum_with_max_length_is_indexable_on_mysql() {
        let col = SnapshotColumn {
            type_name: "travel_contracts::enums::LinkReviewKind".into(),
            max_length: Some(32),
            ..Default::default()
        };
        // EF-MYSQL-01: MySQL cannot index unbounded TEXT, so the declared
        // length must select VARCHAR(n).
        assert_eq!(MigrationDialect::MySql.map_column_type(&col), "VARCHAR(32)");
        // PostgreSQL/SQLite keep TEXT (indexable; matches existing databases).
        assert_eq!(MigrationDialect::Postgres.map_column_type(&col), "TEXT");
        assert_eq!(MigrationDialect::Sqlite.map_column_type(&col), "TEXT");
    }

    #[test]
    fn custom_enum_without_length_stays_text_on_mysql() {
        let col = SnapshotColumn {
            type_name: "some::CustomStatus".into(),
            ..Default::default()
        };
        assert_eq!(MigrationDialect::MySql.map_column_type(&col), "TEXT");
    }

    #[test]
    fn explicit_text_with_length_is_promoted_on_mysql() {
        let col = SnapshotColumn {
            type_name: "alloc::string::String".into(),
            column_type: Some("TEXT".into()),
            max_length: Some(64),
            ..Default::default()
        };
        assert_eq!(MigrationDialect::MySql.map_column_type(&col), "VARCHAR(64)");
        // An explicit non-TEXT type still wins verbatim.
        let json = SnapshotColumn {
            type_name: "alloc::string::String".into(),
            column_type: Some("JSON".into()),
            max_length: Some(64),
            ..Default::default()
        };
        assert_eq!(MigrationDialect::MySql.map_column_type(&json), "JSON");
    }

    #[test]
    fn option_wrapped_custom_enum_uses_inner_mapping() {
        let col = SnapshotColumn {
            type_name: "core::option::Option<some::CustomStatus>".into(),
            max_length: Some(16),
            ..Default::default()
        };
        assert_eq!(MigrationDialect::MySql.map_column_type(&col), "VARCHAR(16)");
    }

    #[test]
    fn option_uuid_and_decimal_use_inner_type() {
        let uuid = SnapshotColumn {
            type_name: "core::option::Option<uuid::Uuid>".into(),
            ..Default::default()
        };
        assert_eq!(MigrationDialect::MySql.map_column_type(&uuid), "CHAR(36)");

        let decimal = SnapshotColumn {
            type_name: "core::option::Option<rust_decimal::Decimal>".into(),
            ..Default::default()
        };
        assert_eq!(
            MigrationDialect::MySql.map_column_type(&decimal),
            "DECIMAL(38,18)"
        );
    }
}
