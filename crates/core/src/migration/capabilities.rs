//! Provider capability matrix (upstream defect package E).
//!
//! The migration engine accepts a portable model, but each dialect supports a
//! different subset of it. Rather than discovering that at `CREATE INDEX` time,
//! the engine and the CLI expose this matrix so an installer can warn up front
//! and a code-first dry run can show exactly what will be emitted.
//!
//! The levels are deliberately coarse:
//!
//! * [`SupportLevel::Native`] — emitted as the model declares it;
//! * [`SupportLevel::Emulated`] — emitted with a documented approximation (for
//!   example a filtered unique index on MySQL becomes a functional index);
//! * [`SupportLevel::Unsupported`] — skipped with a warning comment.

use super::MigrationDialect;

/// How a dialect implements one model feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportLevel {
    /// The dialect implements the feature directly.
    Native,
    /// The dialect has no native equivalent; the generator approximates it.
    Emulated,
    /// The dialect cannot express the feature; the generator skips it with a
    /// warning.
    Unsupported,
}

impl SupportLevel {
    /// Stable lower-case label (`native` / `emulated` / `unsupported`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Emulated => "emulated",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Capabilities of one SQL dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    /// The dialect these capabilities describe.
    pub dialect: MigrationDialect,
    /// Custom/contract enum columns. The framework maps them to `VARCHAR(n)`
    /// on MySQL and `TEXT` elsewhere (plus an optional CHECK).
    pub enum_columns: SupportLevel,
    /// Indexes over raw SQL expressions.
    pub expression_indexes: SupportLevel,
    /// Partial indexes (`CREATE INDEX ... WHERE ...`).
    pub partial_indexes: SupportLevel,
    /// Covering indexes (`INCLUDE (...)`).
    pub include_columns: SupportLevel,
    /// `CHECK` constraints.
    pub check_constraints: SupportLevel,
    /// MySQL table options (`ENGINE` / `DEFAULT CHARSET` / `COLLATE`).
    pub table_options: SupportLevel,
    /// Table / column comments.
    pub comments: SupportLevel,
    /// Generated (computed) columns.
    pub generated_columns: SupportLevel,
    /// Composite foreign keys.
    pub composite_foreign_keys: SupportLevel,
    /// `ALTER TABLE` for a column type/nullability change (SQLite rebuilds the
    /// whole table instead).
    pub alter_column: SupportLevel,
}

impl ProviderCapabilities {
    /// Returns the matrix for `dialect`.
    pub fn for_dialect(dialect: MigrationDialect) -> Self {
        match dialect {
            MigrationDialect::Postgres => Self {
                dialect,
                enum_columns: SupportLevel::Emulated,
                expression_indexes: SupportLevel::Native,
                partial_indexes: SupportLevel::Native,
                include_columns: SupportLevel::Native,
                check_constraints: SupportLevel::Native,
                table_options: SupportLevel::Unsupported,
                comments: SupportLevel::Native,
                generated_columns: SupportLevel::Native,
                composite_foreign_keys: SupportLevel::Native,
                alter_column: SupportLevel::Native,
            },
            MigrationDialect::MySql => Self {
                dialect,
                enum_columns: SupportLevel::Emulated,
                // Functional key parts require MySQL 8.0.13+.
                expression_indexes: SupportLevel::Native,
                // Unique filters become a functional CASE index; non-unique
                // filters have no equivalent.
                partial_indexes: SupportLevel::Emulated,
                include_columns: SupportLevel::Unsupported,
                check_constraints: SupportLevel::Native,
                table_options: SupportLevel::Native,
                comments: SupportLevel::Native,
                generated_columns: SupportLevel::Native,
                composite_foreign_keys: SupportLevel::Native,
                alter_column: SupportLevel::Native,
            },
            MigrationDialect::Sqlite => Self {
                dialect,
                enum_columns: SupportLevel::Emulated,
                expression_indexes: SupportLevel::Native,
                partial_indexes: SupportLevel::Native,
                include_columns: SupportLevel::Unsupported,
                check_constraints: SupportLevel::Native,
                table_options: SupportLevel::Unsupported,
                comments: SupportLevel::Unsupported,
                generated_columns: SupportLevel::Native,
                composite_foreign_keys: SupportLevel::Native,
                // SQLite has no ALTER COLUMN; the runtime diff rebuilds the
                // table atomically instead.
                alter_column: SupportLevel::Emulated,
            },
        }
    }

    /// Multi-line human-readable matrix (used by the CLI dry run).
    pub fn report(&self) -> String {
        let rows = [
            ("enum columns", self.enum_columns),
            ("expression indexes", self.expression_indexes),
            ("partial indexes", self.partial_indexes),
            ("INCLUDE columns", self.include_columns),
            ("CHECK constraints", self.check_constraints),
            ("table options", self.table_options),
            ("comments", self.comments),
            ("generated columns", self.generated_columns),
            ("composite foreign keys", self.composite_foreign_keys),
            ("ALTER COLUMN", self.alter_column),
        ];
        let mut out = format!("{} capabilities:\n", self.dialect_name());
        for (feature, level) in rows {
            out.push_str(&format!("  {feature:<24} {}\n", level.as_str()));
        }
        out
    }

    /// Display name of the dialect.
    pub fn dialect_name(&self) -> &'static str {
        match self.dialect {
            MigrationDialect::Postgres => "postgres",
            MigrationDialect::MySql => "mysql",
            MigrationDialect::Sqlite => "sqlite",
        }
    }
}

/// Returns the capability matrix for `dialect`.
pub fn capabilities(dialect: MigrationDialect) -> ProviderCapabilities {
    ProviderCapabilities::for_dialect(dialect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_reports_emulated_partial_indexes_and_no_include() {
        let caps = capabilities(MigrationDialect::MySql);
        assert_eq!(caps.partial_indexes, SupportLevel::Emulated);
        assert_eq!(caps.include_columns, SupportLevel::Unsupported);
        assert_eq!(caps.table_options, SupportLevel::Native);
    }

    #[test]
    fn postgres_has_native_partial_and_include_indexes() {
        let caps = capabilities(MigrationDialect::Postgres);
        assert_eq!(caps.partial_indexes, SupportLevel::Native);
        assert_eq!(caps.include_columns, SupportLevel::Native);
        assert_eq!(caps.table_options, SupportLevel::Unsupported);
    }

    #[test]
    fn sqlite_reports_rebuild_based_alter_column() {
        let caps = capabilities(MigrationDialect::Sqlite);
        assert_eq!(caps.alter_column, SupportLevel::Emulated);
        assert_eq!(caps.comments, SupportLevel::Unsupported);
    }

    #[test]
    fn report_lists_every_feature() {
        let report = capabilities(MigrationDialect::MySql).report();
        assert!(report.contains("expression indexes"));
        assert!(report.contains("CHECK constraints"));
        assert!(report.contains("emulated"));
    }
}
