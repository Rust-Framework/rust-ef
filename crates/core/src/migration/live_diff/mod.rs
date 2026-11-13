//! Runtime model diffing for installed databases (EF-9).
//!
//! A model migration (`MigrationEngine::generate`) diffs two *snapshots*: it
//! knows the schema the previous migration created, so it can emit `CREATE`
//! and `ALTER` statements without looking at a database. A real upgrade is
//! different: the database may have drifted, may have been created by an older
//! build, and the model — not the previous snapshot — is the source of truth.
//!
//! This module closes that gap:
//!
//! 1. The provider introspects the live database into [`DbTable`] values
//!    (`crate::schema`).
//! 2. [`MigrationEngine::diff`] compares the model snapshot against those
//!    tables and produces a [`MigrationDiff`]: executable statements plus
//!    structured warnings plus the list of tables that will be rebuilt.
//! 3. [`MigrationEngine::apply_diff`](super::engine::MigrationEngine) applies
//!    the plan atomically.
//!
//! # Safety model
//!
//! Runtime diffing is deliberately conservative:
//!
//! - **Additive changes are generated.** Missing tables, columns, indexes,
//!   unique indexes, `CHECK` constraints and foreign keys are created.
//! - **Destructive changes are never executed.** Tables/columns/indexes/
//!   constraints that exist in the database but not in the model are reported
//!   as [`WarningKind::SkippedRemoval`] warnings; use a regular migration if a
//!   removal is intended.
//! - **Unknown introspection is not guessed.** `DbTable::checks` and
//!   `DbTable::triggers` are `Option`; `None` means the provider could not
//!   report them. In that case a model `CHECK` that cannot be verified is
//!   reported as [`WarningKind::ConstraintNotIntrospected`] instead of being
//!   duplicated or silently ignored.
//! - **SQLite table rebuilds are atomic.** SQLite cannot `ALTER` constraints
//!   or column types, so such changes rebuild the table inside a single
//!   transaction with foreign keys disabled and `PRAGMA foreign_key_check`
//!   before commit. The rebuild list and warnings are part of the returned
//!   plan, so an installer can print an upgrade report.

mod compare;
mod rebuild;
mod render;

use crate::metadata::EntityTypeMeta;
use crate::migration::dialect::MigrationDialect;
use crate::migration::history::{MIGRATION_HISTORY_TABLE, PRODUCT_VERSION};
use crate::migration::types::Migration;
use crate::schema::DbTable;

use super::engine::MigrationEngine;
use super::types::ModelSnapshot;

/// First line of the `down_sql` of a [`Migration`] produced by
/// [`MigrationDiff::into_migration`].
///
/// A runtime diff has no safe automatic down migration (dropping constraints or
/// rebuilding tables can lose data), so
/// [`MigrationEngine::revert`](super::engine::MigrationEngine::revert) detects
/// this marker and returns a typed error instead of pretending to roll back.
pub(crate) const RUNTIME_DIFF_DOWN_MARKER: &str =
    "-- rust-ef runtime diff: no automatic down migration";

/// Severity of a [`DiffWarning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WarningSeverity {
    /// Informational: the plan is executable, but the operator should know.
    Info,
    /// The change is skipped or needs a manual follow-up.
    Warning,
}

impl WarningSeverity {
    /// Lower-case label used by [`MigrationDiff::report`].
    pub fn as_str(&self) -> &'static str {
        match self {
            WarningSeverity::Info => "info",
            WarningSeverity::Warning => "warning",
        }
    }
}

/// Machine-readable category of a [`DiffWarning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningKind {
    /// An object exists in the live database but not in the model and is not
    /// dropped automatically (destructive change).
    SkippedRemoval,
    /// The provider could not introspect `CHECK` constraints, so model checks
    /// could not be verified or applied.
    ConstraintNotIntrospected,
    /// An existing index has the model's name but a different definition.
    IndexDefinitionMismatch,
    /// A `NOT NULL` column is added without a default; existing rows cannot be
    /// backfilled automatically.
    NotNullColumnWithoutDefault,
    /// The dialect cannot express the change and no rebuild path applies.
    UnsupportedChange,
    /// A change requires a manual follow-up (e.g. primary-key change on
    /// PostgreSQL/MySQL, live trigger metadata unavailable during a rebuild).
    ManualAction,
}

impl WarningKind {
    /// Stable snake-case identifier for logs/telemetry.
    pub fn as_str(&self) -> &'static str {
        match self {
            WarningKind::SkippedRemoval => "skipped_removal",
            WarningKind::ConstraintNotIntrospected => "constraint_not_introspected",
            WarningKind::IndexDefinitionMismatch => "index_definition_mismatch",
            WarningKind::NotNullColumnWithoutDefault => "not_null_column_without_default",
            WarningKind::UnsupportedChange => "unsupported_change",
            WarningKind::ManualAction => "manual_action",
        }
    }
}

/// One structured warning produced by a runtime model diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffWarning {
    /// How much attention the operator should pay.
    pub severity: WarningSeverity,
    /// Machine-readable category.
    pub kind: WarningKind,
    /// Table the warning refers to, when applicable.
    pub table: Option<String>,
    /// Human-readable explanation (safe to print in an upgrade report).
    pub message: String,
}

impl DiffWarning {
    /// Creates an informational warning.
    pub fn info(
        kind: WarningKind,
        table: Option<String>,
        message: impl Into<String>,
    ) -> DiffWarning {
        DiffWarning {
            severity: WarningSeverity::Info,
            kind,
            table,
            message: message.into(),
        }
    }

    /// Creates a warning that requires operator attention.
    pub fn warning(
        kind: WarningKind,
        table: Option<String>,
        message: impl Into<String>,
    ) -> DiffWarning {
        DiffWarning {
            severity: WarningSeverity::Warning,
            kind,
            table,
            message: message.into(),
        }
    }
}

/// Description of one SQLite table rebuild contained in a [`MigrationDiff`].
///
/// Exposed so an installer/upgrade screen can report exactly what will happen
/// to existing data before [`MigrationEngine::apply_diff`] runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableRebuild {
    /// Table being rebuilt.
    pub table: String,
    /// Columns copied from the old table into the new one (by name).
    pub columns_copied: Vec<String>,
    /// Model columns that do not exist in the live table (they are created with
    /// their default value; existing rows take that default or NULL).
    pub new_columns: Vec<String>,
    /// New `NOT NULL` columns without a default. The rebuild fails atomically
    /// when the table is non-empty; `apply_diff` rejects the plan up-front.
    pub not_null_columns_without_default: Vec<String>,
    /// Whether live triggers were reported and will be recreated afterwards.
    pub triggers_preserved: bool,
}

/// A runtime model diff: executable statements + structured reporting.
///
/// This intentionally does not reuse [`super::types::Migration`]: that type
/// carries an `id`/`down_sql` for the file-based migration workflow, while a
/// runtime diff has no versioned identity and no safe automatic rollback.
#[derive(Debug, Clone, Default)]
pub struct MigrationDiff {
    /// Executable SQL statements, in execution order (no trailing semicolons).
    pub statements: Vec<String>,
    /// Structured warnings/notes for the operator.
    pub warnings: Vec<DiffWarning>,
    /// SQLite tables that will be rebuilt, with data-preservation details.
    pub rebuilds: Vec<TableRebuild>,
}

impl MigrationDiff {
    /// Whether the database already matches the model.
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }

    /// Number of tables the plan rebuilds.
    pub fn rebuild_count(&self) -> usize {
        self.rebuilds.len()
    }

    /// Renders the executable statements as one SQL script.
    ///
    /// Comments are not included; use [`Self::report`] for the human-readable
    /// summary.
    pub fn up_sql(&self) -> String {
        let mut out = String::new();
        for statement in &self.statements {
            let statement = statement.trim_end();
            if statement.is_empty() {
                continue;
            }
            out.push_str(statement);
            if !statement.ends_with(';') {
                out.push(';');
            }
            out.push('\n');
        }
        out
    }

    /// Renders a human-readable upgrade report (tables rebuilt, warnings).
    ///
    /// Suitable for installer logs / upgrade confirmation screens.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("rust-ef runtime schema diff\n");
        out.push_str(&format!("  statements: {}\n", self.statements.len()));
        out.push_str(&format!("  table rebuilds: {}\n", self.rebuilds.len()));
        for rebuild in &self.rebuilds {
            out.push_str(&format!(
                "    - {}: {} column(s) copied, {} new column(s), triggers preserved: {}\n",
                rebuild.table,
                rebuild.columns_copied.len(),
                rebuild.new_columns.len(),
                rebuild.triggers_preserved
            ));
            if !rebuild.not_null_columns_without_default.is_empty() {
                out.push_str(&format!(
                    "      NOT NULL without default: {}\n",
                    rebuild.not_null_columns_without_default.join(", ")
                ));
            }
        }
        out.push_str(&format!("  warnings: {}\n", self.warnings.len()));
        for warning in &self.warnings {
            out.push_str(&format!(
                "    [{}:{}] {}\n",
                warning.severity.as_str(),
                warning.kind.as_str(),
                warning.message
            ));
        }
        out
    }

    /// Converts the plan into a versioned [`Migration`].
    ///
    /// For teams that want the `__ef_migrations_history` row and the existing
    /// `apply` flow, the returned migration embeds the plan statements plus the
    /// history `INSERT`; apply it with
    /// [`MigrationEngine::apply`](super::engine::MigrationEngine::apply).
    ///
    /// # No automatic down migration
    ///
    /// A runtime diff has no safe inverse: dropping constraints or rebuilding
    /// tables can lose data, and the plan does not capture the previous values.
    /// `down_sql` therefore only contains the
    /// [`RUNTIME_DIFF_DOWN_MARKER`] comment, and
    /// [`MigrationEngine::revert`](super::engine::MigrationEngine::revert)
    /// returns a typed error for such a migration (it must not silently remove
    /// the history row while leaving the schema upgraded). Restore from a backup
    /// or write a versioned migration with an explicit down script.
    ///
    /// Runtime upgrades should normally use
    /// [`MigrationEngine::apply_diff`](super::engine::MigrationEngine::apply_diff),
    /// which keeps the structured warnings/rebuild report.
    pub fn into_migration(&self, id: impl Into<String>, dialect: MigrationDialect) -> Migration {
        let id = id.into();
        let mut up_sql = self.up_sql();
        up_sql.push_str(&format!(
            "INSERT INTO {}(migration_id, product_version) VALUES ('{}', '{}');\n",
            dialect.quote(MIGRATION_HISTORY_TABLE),
            id.replace('\'', "''"),
            PRODUCT_VERSION
        ));
        Migration {
            id: id.clone(),
            description: id,
            up_sql,
            down_sql: format!(
                "{RUNTIME_DIFF_DOWN_MARKER}: restore from a backup or write a versioned migration.\n"
            ),
        }
    }
}

/// Whether `migration` was produced by [`MigrationDiff::into_migration`] and
/// therefore has no executable down script.
pub(crate) fn is_runtime_diff_migration(migration: &Migration) -> bool {
    migration
        .down_sql
        .trim_start()
        .starts_with(RUNTIME_DIFF_DOWN_MARKER)
}

impl MigrationEngine {
    /// Diffs a model snapshot against a live database schema.
    ///
    /// The returned [`MigrationDiff`] contains the statements needed to bring
    /// the database in line with the model, plus structured warnings for
    /// anything that is intentionally not applied. Apply it with
    /// [`Self::apply_diff`](MigrationEngine::apply_diff).
    ///
    /// The diff itself is pure (no database access); introspect immediately
    /// before diffing so the plan matches the current database.
    pub fn diff(&self, model: &ModelSnapshot, live: &[DbTable]) -> MigrationDiff {
        render::plan(self, model, live)
    }

    /// Diffs entity type metadata against a live database schema.
    ///
    /// Convenience wrapper over [`Self::diff`] for installers/upgrade flows
    /// that only have the registered metadata at hand.
    pub fn diff_metas(&self, entity_types: &[EntityTypeMeta], live: &[DbTable]) -> MigrationDiff {
        let model = self.create_snapshot("__runtime_diff__", entity_types);
        self.diff(&model, live)
    }
}

#[cfg(test)]
mod tests;
