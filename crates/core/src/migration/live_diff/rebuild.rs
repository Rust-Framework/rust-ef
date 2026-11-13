//! SQLite table rebuild: shadow table + data copy + rename.
//!
//! SQLite cannot `ALTER TABLE` a `CHECK` constraint, a foreign key or a column
//! type. The supported workaround is the documented 12-step rebuild recipe; the
//! important part for an installed database is *data preservation*, so the plan
//! copies every common column explicitly, recreates indexes and triggers, and
//! is executed inside one transaction (see `diff_apply`):
//!
//! ```sql
//! CREATE TABLE "__ef_rebuild_children" (... model definition ...);
//! INSERT INTO "__ef_rebuild_children" ("id", "parent_id") SELECT "id", "parent_id" FROM "children";
//! DROP TABLE "children";
//! ALTER TABLE "__ef_rebuild_children" RENAME TO "children";
//! CREATE INDEX ...;   -- model indexes + live-only indexes
//! CREATE TRIGGER ...; -- triggers reported by introspection
//! ```
//!
//! # Safety rules
//!
//! - A rebuild is refused when the live table has columns the model no longer
//!   declares: copying only common columns would silently drop data. The
//!   operator gets a [`WarningKind::ManualAction`] warning instead.
//! - Columns that exist in the model but not in the live table are left out of
//!   the copy so the database fills in their default. `NOT NULL` columns
//!   without a default make the plan fail on a non-empty table; `apply_diff`
//!   rejects that plan before touching the database.
//! - Live triggers are recreated from the introspected SQL. When the provider
//!   could not introspect triggers (`None`), the rebuild still runs but a
//!   warning states that triggers must be recreated manually.

use crate::migration::diff::fk_target;
use crate::migration::engine::MigrationEngine;
use crate::migration::history::split_sql_statements;
use crate::migration::types::SnapshotEntityType;
use crate::schema::{DbIndex, DbTable};

use super::compare::{expected_indexes, index_matches, is_implicit_index, TableDelta};
use super::{DiffWarning, TableRebuild, WarningKind};

/// Result of planning one SQLite table rebuild.
pub(crate) enum RebuildOutcome {
    /// The table can be rebuilt; statements are ready to execute.
    Planned {
        statements: Vec<String>,
        rebuild: TableRebuild,
        warnings: Vec<DiffWarning>,
    },
    /// The rebuild would lose data; nothing is emitted.
    Refused(DiffWarning),
}

/// Builds the rebuild plan for a delta that needs one on SQLite.
pub(crate) fn plan_rebuild(
    engine: &MigrationEngine,
    delta: &TableDelta<'_>,
    live_tables: &[DbTable],
) -> RebuildOutcome {
    let table = delta.table.clone();
    let Some(et) = delta.model else {
        return RebuildOutcome::Refused(refused(
            &table,
            "the model definition for the table is missing",
        ));
    };
    let Some(live_table) = delta.live else {
        return RebuildOutcome::Refused(refused(
            &table,
            "the live table definition is missing",
        ));
    };
    if et.columns.is_empty() {
        return RebuildOutcome::Refused(refused(
            &table,
            "the model declares no columns for the table",
        ));
    }

    // Refuse when the rebuild would drop live columns (data loss).
    let dropped_columns: Vec<String> = live_table
        .columns
        .iter()
        .filter(|live_column| {
            !et.columns
                .iter()
                .any(|model_column| model_column.column_name.eq_ignore_ascii_case(&live_column.name))
        })
        .map(|live_column| live_column.name.clone())
        .collect();
    if !dropped_columns.is_empty() {
        return RebuildOutcome::Refused(refused(
            &table,
            &format!(
                "the live table contains column(s) {} that are not declared in the model, so the \
                 rebuild would drop their data; apply a versioned migration (or rename/backfill) first",
                dropped_columns.join(", ")
            ),
        ));
    }

    let temporary = temporary_name(&table, et, live_tables);
    let mut statements: Vec<String> = Vec::new();
    statements.push(create_table_sql(engine, &temporary, et));
    if let Some(copy) = copy_data_sql(engine, &temporary, &table, et, live_table) {
        statements.push(copy);
    }
    statements.push(format!(
        "DROP TABLE {}",
        engine.dialect.quote(&table)
    ));
    statements.push(format!(
        "ALTER TABLE {} RENAME TO {}",
        engine.dialect.quote(&temporary),
        engine.dialect.quote(&table)
    ));

    let model_indexes = expected_indexes(et);
    for index in &model_indexes {
        // `create_declared_index_sql` may prepend a dialect warning comment for
        // unsupported options; keep only executable statements.
        let sql = engine.create_declared_index_sql(&table, index);
        statements.extend(split_comment_free(&sql, engine));
    }
    let live_primary_key: Vec<String> = live_table
        .columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| column.name.clone())
        .collect();
    for live_index in &live_table.indexes {
        if is_implicit_index(live_index, &live_primary_key)
            || model_indexes
                .iter()
                .any(|index| index_matches(index, live_index) || index.name.eq_ignore_ascii_case(&live_index.name))
        {
            continue;
        }
        statements.push(create_index_from_live(engine, &table, live_index));
    }

    let mut warnings = Vec::new();
    let triggers_preserved = match &live_table.triggers {
        Some(triggers) => {
            for trigger in triggers {
                let sql = trigger.sql.trim().trim_end_matches(';').trim_end();
                if !sql.is_empty() {
                    statements.push(sql.to_string());
                }
            }
            true
        }
        None => {
            warnings.push(DiffWarning::warning(
                WarningKind::ManualAction,
                Some(table.clone()),
                format!(
                    "the provider could not introspect triggers on table '{table}'; the rebuild \
                     drops them and they must be recreated manually"
                ),
            ));
            false
        }
    };

    let columns_copied: Vec<String> = et
        .columns
        .iter()
        .filter(|column| {
            column.computed_sql.is_none()
                && live_table
                    .columns
                    .iter()
                    .any(|live_column| live_column.name.eq_ignore_ascii_case(&column.column_name))
        })
        .map(|column| column.column_name.clone())
        .collect();
    let new_columns: Vec<String> = et
        .columns
        .iter()
        .filter(|column| {
            !live_table
                .columns
                .iter()
                .any(|live_column| live_column.name.eq_ignore_ascii_case(&column.column_name))
        })
        .map(|column| column.column_name.clone())
        .collect();
    let not_null_columns_without_default: Vec<String> = et
        .columns
        .iter()
        .filter(|column| {
            new_columns
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&column.column_name))
                && (column.is_required || column.is_primary_key)
                && column.default_value.is_none()
                && column.computed_sql.is_none()
        })
        .map(|column| column.column_name.clone())
        .collect();

    RebuildOutcome::Planned {
        statements,
        rebuild: TableRebuild {
            table,
            columns_copied,
            new_columns,
            not_null_columns_without_default,
            triggers_preserved,
        },
        warnings,
    }
}

fn refused(table: &str, reason: &str) -> DiffWarning {
    DiffWarning::warning(
        WarningKind::ManualAction,
        Some(table.to_string()),
        format!("cannot rebuild table '{table}': {reason}"),
    )
}

/// A deterministic shadow-table name that is unique against the live schema.
fn temporary_name(table: &str, et: &SnapshotEntityType, live_tables: &[DbTable]) -> String {
    let base = format!("__ef_rebuild_{}", table.replace('.', "_"));
    let mut candidate = base.clone();
    let mut counter = 0usize;
    loop {
        let collides = live_tables
            .iter()
            .any(|live| live.name.eq_ignore_ascii_case(&candidate))
            || et.table_name.eq_ignore_ascii_case(&candidate);
        if !collides {
            return candidate;
        }
        counter += 1;
        candidate = format!("{base}_{counter}");
    }
}

/// Renders the model's table definition for the shadow table.
fn create_table_sql(engine: &MigrationEngine, table: &str, et: &SnapshotEntityType) -> String {
    let q = |identifier: &str| engine.dialect.quote(identifier);
    let mut parts: Vec<String> = et
        .columns
        .iter()
        .map(|column| engine.render_column_definition(column))
        .collect();

    let primary_key: Vec<&str> = et
        .columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| column.column_name.as_str())
        .collect();
    if primary_key.len() == 1 {
        parts.push(format!("PRIMARY KEY ({})", q(primary_key[0])));
    } else if primary_key.len() > 1 {
        let columns: Vec<String> = primary_key.iter().map(|column| q(column)).collect();
        parts.push(format!("PRIMARY KEY ({})", columns.join(", ")));
    }

    for check in &et.checks {
        parts.push(format!(
            "CONSTRAINT {} CHECK ({})",
            q(&check.name),
            check.expression
        ));
    }

    for column in &et.columns {
        let Some((referenced_table, referenced_column)) = fk_target(column) else {
            continue;
        };
        let name = MigrationEngine::foreign_key_name(
            &et.table_name,
            &column.column_name,
            &referenced_table,
        );
        let on_delete = column
            .fk_on_delete
            .as_deref()
            .map(|clause| format!(" ON DELETE {clause}"))
            .unwrap_or_default();
        parts.push(format!(
            "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}){}",
            q(&name),
            q(&column.column_name),
            q(&referenced_table),
            q(&referenced_column),
            on_delete
        ));
    }

    for foreign_key in &et.foreign_keys {
        parts.push(engine.composite_fk_clause(foreign_key));
    }

    format!("CREATE TABLE {} (\n    {}\n)", q(table), parts.join(",\n    "))
}

/// `INSERT INTO shadow (common columns) SELECT common columns FROM old`.
fn copy_data_sql(
    engine: &MigrationEngine,
    temporary: &str,
    table: &str,
    et: &SnapshotEntityType,
    live_table: &DbTable,
) -> Option<String> {
    let q = |identifier: &str| engine.dialect.quote(identifier);
    let columns: Vec<String> = et
        .columns
        .iter()
        .filter(|column| {
            column.computed_sql.is_none()
                && live_table
                    .columns
                    .iter()
                    .any(|live_column| live_column.name.eq_ignore_ascii_case(&column.column_name))
        })
        .map(|column| q(&column.column_name))
        .collect();
    if columns.is_empty() {
        return None;
    }
    let list = columns.join(", ");
    Some(format!(
        "INSERT INTO {} ({list}) SELECT {list} FROM {}",
        q(temporary),
        q(table)
    ))
}

/// Recreates an index that exists in the live database but not in the model, so
/// the rebuild does not silently drop it.
fn create_index_from_live(engine: &MigrationEngine, table: &str, index: &DbIndex) -> String {
    let q = |identifier: &str| engine.dialect.quote(identifier);
    let unique = if index.is_unique { "UNIQUE " } else { "" };
    let columns: Vec<String> = index
        .columns
        .iter()
        .map(|column| {
            if column.is_expression {
                column.value.clone()
            } else {
                q(&column.value)
            }
        })
        .collect();
    let filter = index
        .filter
        .as_deref()
        .map(|filter| format!(" WHERE {filter}"))
        .unwrap_or_default();
    format!(
        "CREATE {unique}INDEX {} ON {} ({}){filter}",
        q(&index.name),
        q(table),
        columns.join(", ")
    )
}

/// Keeps only executable statements from a generated (possibly commented) SQL
/// fragment.
fn split_comment_free(sql: &str, engine: &MigrationEngine) -> Vec<String> {
    split_sql_statements(sql, engine.dialect)
        .into_iter()
        .map(|statement| statement.trim().to_string())
        .filter(|statement| !statement.is_empty())
        .collect()
}
