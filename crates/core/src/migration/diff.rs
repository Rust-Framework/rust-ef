//! Model diffing helpers — detect table/column/FK/index changes between snapshots.

use super::schema_change::SchemaChange;
use super::types::{SnapshotColumn, SnapshotEntityType};
use crate::metadata::{EntityTypeMeta, NavigationKind, NavigationMeta};
use std::collections::{HashMap, HashSet};

pub(super) fn fk_target(col: &SnapshotColumn) -> Option<(String, String)> {
    if !col.is_foreign_key {
        return None;
    }
    Some((
        col.fk_referenced_table.clone()?,
        col.fk_referenced_column.clone()?,
    ))
}

/// Standard index name used by CreateIndex/DropIndex SQL generation.
pub(super) fn index_name(table: &str, column: &str) -> String {
    format!("ix_{}_{}", table, column)
}

pub(super) fn fk_reference_for_property(
    meta: &EntityTypeMeta,
    column_name: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    for nav in &meta.navigations {
        if nav.kind != NavigationKind::BelongsTo {
            continue;
        }
        let matches = nav
            .fk_column
            .as_ref()
            .is_some_and(|fk| fk.as_ref() == column_name);
        if matches {
            let on_delete = resolve_fk_on_delete_clause(nav, meta);
            return (
                nav.related_table.as_ref().map(|s| s.to_string()),
                nav.referenced_key_column.as_ref().map(|s| s.to_string()),
                on_delete,
            );
        }
    }
    (None, None, None)
}

/// Resolves the `ON DELETE` SQL clause for a FK by checking:
/// 1. The child's BelongsTo navigation `delete_behavior` (if user configured it here)
/// 2. The principal's inverse HasMany navigation via `resolve_delete_behavior`
/// 3. Fallback: nullability of the FK property on the child entity
fn resolve_fk_on_delete_clause(
    belongs_to_nav: &NavigationMeta,
    child_meta: &EntityTypeMeta,
) -> Option<String> {
    use crate::relations::DeleteBehavior;

    // 1. Explicit on the BelongsTo side
    if let Some(b) = belongs_to_nav.delete_behavior {
        return Some(b.to_sql_clause().to_string());
    }

    // 2. Find the principal's HasMany navigation pointing back
    if let Some(meta_fn) = belongs_to_nav.related_entity_meta {
        let principal_meta = meta_fn();
        for principal_nav in &principal_meta.navigations {
            if principal_nav.kind == NavigationKind::HasMany
                && principal_nav.related_type_id == child_meta.type_id
            {
                let behavior = crate::db_context::resolve_delete_behavior(principal_nav);
                return Some(behavior.to_sql_clause().to_string());
            }
        }
    }

    // 3. Fallback: nullability from the FK property's Rust type
    if let Some(fk_prop) = child_meta.properties.iter().find(|p| p.is_foreign_key) {
        let is_nullable = fk_prop.type_name.contains("Option");
        let behavior = if is_nullable {
            DeleteBehavior::Restrict
        } else {
            DeleteBehavior::Cascade
        };
        return Some(behavior.to_sql_clause().to_string());
    }

    None
}

pub(super) fn diff_foreign_keys(
    table: &str,
    old_et: &SnapshotEntityType,
    new_et: &SnapshotEntityType,
) -> Vec<SchemaChange> {
    let mut changes = Vec::new();
    let old_fks: HashMap<&str, (&SnapshotColumn, String, String, Option<String>)> = old_et
        .columns
        .iter()
        .filter_map(|c| {
            let (rt, rc) = fk_target(c)?;
            Some((c.column_name.as_str(), (c, rt, rc, c.fk_on_delete.clone())))
        })
        .collect();
    let new_fks: HashMap<&str, (&SnapshotColumn, String, String, Option<String>)> = new_et
        .columns
        .iter()
        .filter_map(|c| {
            let (rt, rc) = fk_target(c)?;
            Some((c.column_name.as_str(), (c, rt, rc, c.fk_on_delete.clone())))
        })
        .collect();

    let old_names: HashSet<&str> = old_fks.keys().copied().collect();
    let new_names: HashSet<&str> = new_fks.keys().copied().collect();

    for col in new_names.difference(&old_names) {
        let (_, rt, rc, od) = &new_fks[col];
        changes.push(SchemaChange::AddForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: rt.clone(),
            referenced_column: rc.clone(),
            on_delete: od.clone(),
        });
    }

    for col in old_names.difference(&new_names) {
        let (_, rt, _, _) = &old_fks[col];
        changes.push(SchemaChange::DropForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: rt.clone(),
        });
    }

    for col in old_names.intersection(&new_names) {
        let (_, old_rt, old_rc, old_od) = &old_fks[col];
        let (_, new_rt, new_rc, new_od) = &new_fks[col];
        if old_rt != new_rt || old_rc != new_rc || old_od != new_od {
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
                on_delete: new_od.clone(),
            });
        }
    }

    changes
}

/// Compares two columns for structural equality, ignoring index state
/// (`has_index` / `is_unique`). Index changes are handled separately by
/// `diff_indexes` so they emit `CreateIndex`/`DropIndex` instead of
/// `AlterColumn`.
pub(super) fn columns_structurally_equal(a: &SnapshotColumn, b: &SnapshotColumn) -> bool {
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
pub(super) fn diff_indexes(
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
enum IndexKind {
    None,
    NonUnique,
    Unique,
}

fn index_kind(col: &SnapshotColumn) -> IndexKind {
    if col.is_unique {
        IndexKind::Unique
    } else if col.has_index {
        IndexKind::NonUnique
    } else {
        IndexKind::None
    }
}
