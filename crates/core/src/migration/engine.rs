//! `MigrationEngine` — diffs model snapshots and generates migration SQL.
//!
//! This module contains the engine struct, its constructor, and the
//! diff/snapshot-generation logic. SQL generation methods live in
//! `engine_sql.rs`; async execution methods live in `engine_exec.rs`.
//!
//! Rust permits multiple `impl MigrationEngine` blocks across files —
//! method resolution is unaffected by which file an `impl` block resides in.

use std::collections::{HashMap, HashSet};

use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;

use super::diff::{
    columns_structurally_equal, diff_foreign_keys, diff_indexes, fk_reference_for_property,
    fk_target,
};
use super::types::{
    Migration, MigrationDialect, ModelSnapshot, SchemaChange, SnapshotColumn,
    SnapshotEntityType,
};

/// The migration engine — compares old and new model snapshots to generate
/// migration SQL.
pub struct MigrationEngine {
    pub(crate) dialect: MigrationDialect,
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
    ) -> EFResult<Migration> {
        let current_snapshot = self.create_snapshot("__current__", current);

        let changes = match previous_snapshot {
            Some(prev) => self.diff(prev, &current_snapshot),
            None => self.initial_create_with_fks(&current_snapshot),
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
                    .map(|p| {
                        let (fk_table, fk_col) =
                            fk_reference_for_property(et, p.field_name.as_ref());
                        SnapshotColumn {
                            field_name: p.field_name.to_string(),
                            column_name: p.column_name.to_string(),
                            type_name: p.type_name.to_string(),
                            is_primary_key: p.is_primary_key,
                            is_required: p.is_required,
                            is_foreign_key: p.is_foreign_key,
                            max_length: p.max_length,
                            is_auto_increment: p.is_auto_increment,
                            fk_referenced_table: fk_table,
                            fk_referenced_column: fk_col,
                            has_index: p.has_index,
                            is_unique: p.is_unique,
                        }
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

    pub(crate) fn initial_create(&self, current: &ModelSnapshot) -> Vec<SchemaChange> {
        current
            .entity_types
            .iter()
            .map(|et| SchemaChange::CreateTable {
                table: et.table_name.clone(),
                columns: et.columns.clone(),
            })
            .collect()
    }

    pub(crate) fn diff(
        &self,
        old: &ModelSnapshot,
        new: &ModelSnapshot,
    ) -> Vec<SchemaChange> {
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
            Self::append_create_table_fks(&mut changes, &et.table_name, &et.columns);
            Self::append_create_table_indexes(&mut changes, &et.table_name, &et.columns);
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

            // Added columns (with indexes if configured)
            for col_name in new_col_names.difference(&old_col_names) {
                let col = new_cols[col_name];
                changes.push(SchemaChange::AddColumn {
                    table: table.clone(),
                    column: (*col).clone(),
                });
                changes.extend(diff_indexes(table, &SnapshotColumn::default(), col));
            }

            // Foreign keys (after add-column, before drop-column)
            changes.extend(diff_foreign_keys(table, old_et, new_et));

            // Dropped columns
            for col_name in old_col_names.difference(&new_col_names) {
                changes.push(SchemaChange::DropColumn {
                    table: table.clone(),
                    column_name: col_name.to_string(),
                });
            }

            // Altered columns and index changes (present in both)
            for col_name in old_col_names.intersection(&new_col_names) {
                let old_col = old_cols[col_name];
                let new_col = new_cols[col_name];

                // Index state changes → CreateIndex/DropIndex
                changes.extend(diff_indexes(table, old_col, new_col));

                // Structural changes (excluding index fields) → AlterColumn
                if !columns_structurally_equal(old_col, new_col) {
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

    pub(crate) fn append_create_table_fks(
        changes: &mut Vec<SchemaChange>,
        table: &str,
        columns: &[SnapshotColumn],
    ) {
        for col in columns {
            if let Some((ref_table, ref_col)) = fk_target(col) {
                changes.push(SchemaChange::AddForeignKey {
                    table: table.to_string(),
                    column: col.column_name.clone(),
                    referenced_table: ref_table,
                    referenced_column: ref_col,
                });
            }
        }
    }

    /// Emits `CreateIndex` changes for indexed columns in a newly created table.
    pub(crate) fn append_create_table_indexes(
        changes: &mut Vec<SchemaChange>,
        table: &str,
        columns: &[SnapshotColumn],
    ) {
        for col in columns {
            if col.is_unique {
                changes.push(SchemaChange::CreateIndex {
                    table: table.to_string(),
                    column: col.column_name.clone(),
                    is_unique: true,
                });
            } else if col.has_index {
                changes.push(SchemaChange::CreateIndex {
                    table: table.to_string(),
                    column: col.column_name.clone(),
                    is_unique: false,
                });
            }
        }
    }
}
