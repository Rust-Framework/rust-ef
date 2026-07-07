//! Diff helper functions for comparing model snapshots.
//!
//! These free functions detect schema changes between old and new
//! `SnapshotColumn` values: foreign-key additions/removals, column type
//! changes, and index additions/removals. They are consumed by
//! `engine.rs` to build the `Vec<SchemaChange>` fed into SQL generation.

use std::collections::{HashMap, HashSet};

use crate::metadata::{EntityTypeMeta, NavigationKind};

use super::types::{SchemaChange, SnapshotColumn, SnapshotEntityType};

pub(crate) fn fk_target(col: &SnapshotColumn) -> Option<(String, String)> {
    if !col.is_foreign_key {
        return None;
    }
    Some((
        col.fk_referenced_table.clone()?,
        col.fk_referenced_column.clone()?,
    ))
}

/// Standard index name used by CreateIndex/DropIndex SQL generation.
pub(crate) fn index_name(table: &str, column: &str) -> String {
    format!("ix_{}_{}", table, column)
}

pub(crate) fn fk_reference_for_property(
    meta: &EntityTypeMeta,
    field_name: &str,
) -> (Option<String>, Option<String>) {
    for nav in &meta.navigations {
        if nav.kind != NavigationKind::BelongsTo {
            continue;
        }
        let matches = nav
            .foreign_key_field
            .as_ref()
            .is_some_and(|fk| fk.as_ref() == field_name);
        if matches {
            return (
                nav.related_table.as_ref().map(|s| s.to_string()),
                nav.referenced_key_column.as_ref().map(|s| s.to_string()),
            );
        }
    }
    (None, None)
}

pub(crate) fn diff_foreign_keys(
    table: &str,
    old_et: &SnapshotEntityType,
    new_et: &SnapshotEntityType,
) -> Vec<SchemaChange> {
    let mut changes = Vec::new();
    let old_fks: HashMap<&str, (&SnapshotColumn, String, String)> = old_et
        .columns
        .iter()
        .filter_map(|c| {
            let (rt, rc) = fk_target(c)?;
            Some((c.column_name.as_str(), (c, rt, rc)))
        })
        .collect();
    let new_fks: HashMap<&str, (&SnapshotColumn, String, String)> = new_et
        .columns
        .iter()
        .filter_map(|c| {
            let (rt, rc) = fk_target(c)?;
            Some((c.column_name.as_str(), (c, rt, rc)))
        })
        .collect();

    let old_names: HashSet<&str> = old_fks.keys().copied().collect();
    let new_names: HashSet<&str> = new_fks.keys().copied().collect();

    for col in new_names.difference(&old_names) {
        let (_, rt, rc) = &new_fks[col];
        changes.push(SchemaChange::AddForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: rt.clone(),
            referenced_column: rc.clone(),
        });
    }

    for col in old_names.difference(&new_names) {
        let (_, rt, _) = &old_fks[col];
        changes.push(SchemaChange::DropForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: rt.clone(),
        });
    }

    for col in old_names.intersection(&new_names) {
        let (_, old_rt, old_rc) = &old_fks[col];
        let (_, new_rt, new_rc) = &new_fks[col];
        if old_rt != new_rt || old_rc != new_rc {
            changes.push(SchemaChange::DropForeignKey {
                table: table.to_string(),
                column: (*col).to_string(),
                referenced_table: old_rt.clone(),
            });
            changes.push(SchemaChange::AddForeignKey {
                table: table.to_string(),
                column: (*col).to_string(),
                referenced_table: new_rt.clone(),
                referenced_column: new_rc.clone(),
            });
        }
    }

    changes
}

/// Compares two columns for structural equality, ignoring index state
/// (`has_index` / `is_unique`). Index changes are handled separately by
/// `diff_indexes` so they emit `CreateIndex`/`DropIndex` instead of
/// `AlterColumn`.
pub(crate) fn columns_structurally_equal(a: &SnapshotColumn, b: &SnapshotColumn) -> bool {
    a.field_name == b.field_name
        && a.column_name == b.column_name
        && a.type_name == b.type_name
        && a.is_primary_key == b.is_primary_key
        && a.is_required == b.is_required
        && a.is_foreign_key == b.is_foreign_key
        && a.max_length == b.max_length
        && a.is_auto_increment == b.is_auto_increment
        && a.fk_referenced_table == b.fk_referenced_table
        && a.fk_referenced_column == b.fk_referenced_column
}

/// Returns `CreateIndex`/`DropIndex` changes when a column's index state
/// transitions between None / NonUnique / Unique.
pub(crate) fn diff_indexes(
    table: &str,
    old: &SnapshotColumn,
    new: &SnapshotColumn,
) -> Vec<SchemaChange> {
    let old_kind = index_kind(old);
    let new_kind = index_kind(new);
    if old_kind == new_kind {
        return Vec::new();
    }
    let mut changes = Vec::new();
    if old_kind != IndexKind::None {
        changes.push(SchemaChange::DropIndex {
            table: table.to_string(),
            column: new.column_name.clone(),
            is_unique: old_kind == IndexKind::Unique,
        });
    }
    if new_kind != IndexKind::None {
        changes.push(SchemaChange::CreateIndex {
            table: table.to_string(),
            column: new.column_name.clone(),
            is_unique: new_kind == IndexKind::Unique,
        });
    }
    changes
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexKind {
    None,
    NonUnique,
    Unique,
}

pub(crate) fn index_kind(col: &SnapshotColumn) -> IndexKind {
    if col.is_unique {
        IndexKind::Unique
    } else if col.has_index {
        IndexKind::NonUnique
    } else {
        IndexKind::None
    }
}
