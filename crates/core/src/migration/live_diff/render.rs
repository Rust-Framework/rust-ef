//! Turns detected changes into an executable [`MigrationDiff`].
//!
//! Statements are rendered in a dependency-friendly order
//! (`CREATE TABLE` → column changes → indexes → constraints) so a plan does not
//! need to be re-sorted by the caller. The regular DDL generator is reused: the
//! same `SchemaChange` → SQL pipeline used by `generate` produces the ALTER
//! statements, which keeps dialect behavior (quoting, type mapping, MySQL
//! auto-increment handling, PostgreSQL comments, …) in one place.

use crate::schema::DbTable;

use super::compare::{detect, TableDelta};
use crate::migration::dialect::MigrationDialect;
use crate::migration::history::split_sql_statements;
use crate::migration::schema_change::SchemaChange;
use crate::migration::types::{ModelSnapshot, SnapshotEntityType};
use super::{DiffWarning, MigrationDiff, WarningKind};

use crate::migration::engine::MigrationEngine;

pub(crate) fn plan(
    engine: &MigrationEngine,
    model: &ModelSnapshot,
    live: &[DbTable],
) -> MigrationDiff {
    let (deltas, mut warnings) = detect(model, live, engine.dialect);
    let mut statements: Vec<String> = Vec::new();

    // 1. Tables that do not exist yet. Reuse the exact "fresh database" path so
    //    SQLite gets inline foreign keys and PostgreSQL gets its sequences.
    let new_tables: Vec<&SnapshotEntityType> = deltas
        .iter()
        .filter(|delta| delta.model.is_some() && delta.live.is_none())
        .filter_map(|delta| delta.model)
        .collect();
    if !new_tables.is_empty() {
        let snapshot = ModelSnapshot {
            migration_id: "__runtime_diff__".to_string(),
            entity_types: new_tables.into_iter().cloned().collect(),
        };
        let changes = engine.initial_create_with_fks(&snapshot);
        statements.extend(render_changes(engine, &changes));
    }

    // 2. Changes to existing tables, grouped so constraints are added after the
    //    columns and indexes they depend on.
    let mut column_changes: Vec<SchemaChange> = Vec::new();
    let mut index_changes: Vec<SchemaChange> = Vec::new();
    let mut constraint_changes: Vec<SchemaChange> = Vec::new();

    for delta in deltas.iter().filter(|delta| delta.live.is_some()) {
        if engine.dialect == MigrationDialect::Sqlite && !delta.rebuild_reasons.is_empty() {
            render_rebuild_placeholder(delta, &mut warnings);
            continue;
        }
        for change in &delta.changes {
            match change {
                SchemaChange::AddColumn { .. } | SchemaChange::AlterColumn { .. } => {
                    column_changes.push(change.clone())
                }
                SchemaChange::CreateIndex { .. }
                | SchemaChange::DropIndex { .. }
                | SchemaChange::CreateDeclaredIndex { .. }
                | SchemaChange::DropDeclaredIndex { .. } => index_changes.push(change.clone()),
                _ => constraint_changes.push(change.clone()),
            }
        }
    }

    statements.extend(render_changes(engine, &column_changes));
    statements.extend(render_changes(engine, &index_changes));
    statements.extend(render_changes(engine, &constraint_changes));

    MigrationDiff {
        statements,
        warnings,
        rebuilds: Vec::new(),
    }
}

/// Emits a structured warning for a SQLite table that would need a rebuild.
///
/// The rebuild itself is implemented by the `sqlite_rebuild` module; this
/// placeholder keeps the intermediate stage honest: nothing is applied and the
/// operator is told exactly why.
fn render_rebuild_placeholder(delta: &TableDelta<'_>, warnings: &mut Vec<DiffWarning>) {
    warnings.push(DiffWarning::warning(
        WarningKind::UnsupportedChange,
        Some(delta.table.clone()),
        format!(
            "SQLite cannot apply {count} change(s) to existing table '{table}' without rebuilding it: {reasons}. \
             The table is left untouched; apply a versioned migration or rebuild it manually",
            count = delta.rebuild_reasons.len(),
            table = delta.table,
            reasons = delta.rebuild_reasons.join("; ")
        ),
    ));
}

/// Renders logical changes through the regular DDL generator and splits the
/// script into individual statements, dropping the migration-history INSERT
/// (a runtime diff has no versioned identity).
fn render_changes(engine: &MigrationEngine, changes: &[SchemaChange]) -> Vec<String> {
    if changes.is_empty() {
        return Vec::new();
    }
    let sql = engine.generate_up_sql(changes);
    split_plan(&sql, engine.dialect)
}

fn split_plan(sql: &str, dialect: MigrationDialect) -> Vec<String> {
    split_sql_statements(sql, dialect)
        .into_iter()
        .filter(|statement| {
            !statement
                .to_ascii_uppercase()
                .contains("__EF_MIGRATIONS_HISTORY")
        })
        .map(|statement| statement.trim().to_string())
        .filter(|statement| !statement.is_empty())
        .collect()
}
