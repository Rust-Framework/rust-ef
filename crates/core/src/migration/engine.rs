//! `MigrationEngine` — model snapshot diffing and migration generation.

use super::ddl_expr;
use super::dialect::MigrationDialect;
use super::diff::{
    columns_structurally_equal, diff_foreign_keys, diff_indexes, fk_reference_for_property,
};
use super::schema_change::SchemaChange;
use super::types::{
    Migration, ModelSnapshot, SnapshotCheck, SnapshotColumn, SnapshotEntityType,
    SnapshotForeignKey, SnapshotIndex, SnapshotIndexColumn, SnapshotTableOptions,
};
use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;
use std::collections::{HashMap, HashSet};

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

    /// Generates a migration directly from model snapshots.
    ///
    /// Unlike [`Self::generate`] this preserves snapshot-only details that
    /// cannot be reconstructed from [`EntityTypeMeta`] (e.g. rendered
    /// partial-index predicates). Used by `rust-ef migration add`, which reads
    /// a snapshot file.
    pub fn generate_from_snapshot(
        &self,
        name: &str,
        current: &ModelSnapshot,
        previous_snapshot: &Option<ModelSnapshot>,
    ) -> EFResult<Migration> {
        let changes = match previous_snapshot {
            Some(prev) => self.diff(prev, current),
            None => self.initial_create_with_fks(current),
        };
        Ok(Migration {
            id: name.to_string(),
            description: name.to_string(),
            up_sql: self.generate_up_sql(&changes),
            down_sql: self.generate_down_sql(&changes),
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
            .map(|et| {
                // Maps a Rust field name or a source column name to the final
                // (possibly renamed) database column name.
                let resolve_column = |name: &str| -> String {
                    et.properties
                        .iter()
                        .find(|p| p.column_name == name)
                        .or_else(|| et.properties.iter().find(|p| p.field_name == name))
                        .map(|p| p.column_name.to_string())
                        .unwrap_or_else(|| name.to_string())
                };

                // Explicit single-column foreign keys (scalar `#[foreign_key]`)
                // take precedence over navigation-derived ones.
                let explicit_single: HashMap<String, &crate::metadata::ForeignKeyMeta> = et
                    .foreign_keys
                    .iter()
                    .filter(|fk| fk.columns.len() == 1 && fk.referenced_columns.len() == 1)
                    .map(|fk| (resolve_column(&fk.columns[0]), fk))
                    .collect();

                let foreign_keys = et
                    .foreign_keys
                    .iter()
                    .filter(|fk| !(fk.columns.len() == 1 && fk.referenced_columns.len() == 1))
                    .map(|fk| SnapshotForeignKey {
                        name: fk
                            .name
                            .as_ref()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| fk.derive_name(et.table_name.as_ref())),
                        columns: fk.columns.iter().map(|c| resolve_column(c)).collect(),
                        referenced_table: fk.referenced_table.to_string(),
                        referenced_columns: fk
                            .referenced_columns
                            .iter()
                            .map(|c| c.to_string())
                            .collect(),
                        on_delete: fk.on_delete.as_ref().map(|c| c.to_string()),
                    })
                    .collect();

                let indexes = et
                    .indexes
                    .iter()
                    .map(|idx| SnapshotIndex {
                        name: idx
                            .name
                            .as_ref()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| idx.derive_name(et.table_name.as_ref())),
                        columns: idx
                            .columns
                            .iter()
                            .map(|c| SnapshotIndexColumn {
                                value: match c {
                                    crate::metadata::IndexColumn::Column(name) => {
                                        resolve_column(name)
                                    }
                                    crate::metadata::IndexColumn::Expression(_) => {
                                        c.as_str().to_string()
                                    }
                                },
                                is_expression: c.is_expression(),
                            })
                            .collect(),
                        is_unique: idx.is_unique,
                        filter: idx
                            .filter
                            .as_ref()
                            .map(|f| ddl_expr::render_filter(f, self.dialect)),
                        include: idx.include.iter().map(|c| c.to_string()).collect(),
                    })
                    .collect();

                let checks = et
                    .checks
                    .iter()
                    .map(|c| SnapshotCheck {
                        name: c.name.to_string(),
                        expression: c.expression.to_string(),
                    })
                    .collect();

                let table_options = SnapshotTableOptions {
                    engine: et.table_options.engine.as_ref().map(|s| s.to_string()),
                    charset: et.table_options.charset.as_ref().map(|s| s.to_string()),
                    collate: et.table_options.collate.as_ref().map(|s| s.to_string()),
                };

                SnapshotEntityType {
                    type_name: et.type_name.to_string(),
                    table_name: et.table_name.to_string(),
                    columns: et
                        .properties
                        .iter()
                        .filter(|p| !p.is_not_mapped)
                        .map(|p| {
                            let (nav_table, nav_col, nav_on_delete) =
                                fk_reference_for_property(et, p.column_name.as_ref());
                            let (fk_table, fk_col, fk_on_delete) =
                                match explicit_single.get(p.column_name.as_ref()) {
                                    Some(fk) => (
                                        Some(fk.referenced_table.to_string()),
                                        Some(fk.referenced_columns[0].to_string()),
                                        // The scalar FK wins for the target, but
                                        // `#[on_delete]` on the navigation still
                                        // supplies the delete behavior.
                                        fk.on_delete
                                            .as_ref()
                                            .map(|c| c.to_string())
                                            .or(nav_on_delete),
                                    ),
                                    None => (nav_table, nav_col, nav_on_delete),
                                };
                            // A Fluent `has_foreign_key` (single column) or a
                            // navigation also makes this a FK even without a
                            // `#[foreign_key]` attribute.
                            let is_foreign_key = p.is_foreign_key || fk_table.is_some();
                            SnapshotColumn {
                                field_name: p.field_name.to_string(),
                                column_name: p.column_name.to_string(),
                                type_name: p.type_name.to_string(),
                                is_primary_key: p.is_primary_key,
                                is_required: p.is_required,
                                is_foreign_key,
                                max_length: p.max_length,
                                is_auto_increment: p.is_auto_increment,
                                is_sequence: p.is_sequence,
                                sequence_name: p.sequence_name.as_ref().map(|s| s.to_string()),
                                fk_referenced_table: fk_table,
                                fk_referenced_column: fk_col,
                                has_index: p.has_index,
                                is_unique: p.is_unique,
                                fk_on_delete,
                                default_value: p.default_value.as_ref().map(|s| s.to_string()),
                                column_type: p.column_type.as_ref().map(|s| s.to_string()),
                                precision: p.precision,
                                scale: p.scale,
                                is_fixed_length: p.is_fixed_length,
                                collation: p.collation.as_ref().map(|s| s.to_string()),
                                comment: p.comment.as_ref().map(|s| s.to_string()),
                                computed_sql: p.computed_sql.as_ref().map(|s| s.to_string()),
                                computed_stored: p.computed_stored,
                            }
                        })
                        .collect(),
                    indexes,
                    foreign_keys,
                    checks,
                    table_options,
                    is_view: et.is_view,
                    is_keyless: et.is_keyless,
                    comment: et.comment.as_ref().map(|s| s.to_string()),
                }
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
            .filter(|et| !et.is_view)
            .map(|et| SchemaChange::CreateTable {
                table: et.table_name.clone(),
                columns: et.columns.clone(),
                checks: et.checks.clone(),
                table_options: et.table_options.clone(),
                foreign_keys: et.foreign_keys.clone(),
                comment: et.comment.clone(),
            })
            .collect()
    }

    fn diff(&self, old: &ModelSnapshot, new: &ModelSnapshot) -> Vec<SchemaChange> {
        let mut changes = Vec::new();

        // Build lookup maps by table name
        // Views are excluded from table DDL/diffing entirely.
        let old_tables: HashMap<&str, &SnapshotEntityType> = old
            .entity_types
            .iter()
            .filter(|e| !e.is_view)
            .map(|e| (e.table_name.as_str(), e))
            .collect();
        let new_tables: HashMap<&str, &SnapshotEntityType> = new
            .entity_types
            .iter()
            .filter(|e| !e.is_view)
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
                checks: et.checks.clone(),
                table_options: et.table_options.clone(),
                foreign_keys: et.foreign_keys.clone(),
                comment: et.comment.clone(),
            });
            // SQLite cannot ALTER TABLE ADD CONSTRAINT, so its foreign keys are
            // emitted inline in CREATE TABLE instead.
            if self.dialect != MigrationDialect::Sqlite {
                Self::append_create_table_fks(&mut changes, &et.table_name, &et.columns);
                Self::append_composite_fks(&mut changes, et);
            }
            Self::append_create_table_indexes(&mut changes, &et.table_name, &et.columns);
            Self::append_declared_indexes(&mut changes, et);
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
            changes.extend(super::diff::diff_composite_foreign_keys(
                table, old_et, new_et,
            ));

            // Declarative indexes / CHECK constraints / table options.
            changes.extend(super::diff::diff_declared_indexes(table, old_et, new_et));
            changes.extend(super::diff::diff_checks(table, old_et, new_et));
            changes.extend(super::diff::diff_table_comment(table, old_et, new_et));
            changes.extend(super::diff::diff_table_options(table, old_et, new_et));

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
                        old: Box::new((*old_col).clone()),
                        new: Box::new((*new_col).clone()),
                    });
                }
            }
        }

        changes
    }

    pub(super) fn append_create_table_fks(
        changes: &mut Vec<SchemaChange>,
        table: &str,
        columns: &[SnapshotColumn],
    ) {
        for col in columns {
            if let Some((ref_table, ref_col)) = super::diff::fk_target(col) {
                changes.push(SchemaChange::AddForeignKey {
                    table: table.to_string(),
                    column: col.column_name.clone(),
                    referenced_table: ref_table,
                    referenced_column: ref_col,
                    on_delete: col.fk_on_delete.clone(),
                });
            }
        }
    }

    pub(super) fn append_create_table_indexes(
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

    /// Appends `CreateDeclaredIndex` changes for an entity type's declarative
    /// indexes (composite / partial / expression).
    pub(super) fn append_declared_indexes(
        changes: &mut Vec<SchemaChange>,
        et: &SnapshotEntityType,
    ) {
        for index in &et.indexes {
            changes.push(SchemaChange::CreateDeclaredIndex {
                table: et.table_name.clone(),
                index: index.clone(),
            });
        }
    }

    /// Appends `AddCompositeForeignKey` changes for multi-column foreign keys.
    pub(super) fn append_composite_fks(changes: &mut Vec<SchemaChange>, et: &SnapshotEntityType) {
        for fk in &et.foreign_keys {
            changes.push(SchemaChange::AddCompositeForeignKey {
                table: et.table_name.clone(),
                foreign_key: fk.clone(),
            });
        }
    }
}
