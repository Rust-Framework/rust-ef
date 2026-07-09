//! Cascade save: drain children from principal HasMany navigations, FK fixup
//! after principal PK backfill, and M2M join-row insertion via direct SQL.
//!
//! See `cascade-save-plan.md` for the full design. Key types:
//! - [`DrainedChild`]: a child entity extracted from a principal's HasMany,
//!   with parent linkage metadata.
//! - [`FixupLink`]: records the parent→child association for post-INSERT FK
//!   fixup (one-to-many) or join-row insertion (M2M).

use std::any::{Any, TypeId};

/// A child entity drained from a principal's HasMany navigation, with the
/// metadata needed to fixup its FK after the principal's PK is backfilled.
pub struct DrainedChild {
    /// The principal entity's TypeId (e.g. `Blog`).
    pub parent_type_id: TypeId,
    /// Index of the principal entry in its DbSet (for PK lookup after INSERT).
    pub parent_entry_idx: usize,
    /// The drained child entity, type-erased.
    pub child: Box<dyn Any + Send + Sync>,
    /// The child entity's TypeId (e.g. `Post`).
    pub child_type_id: TypeId,
    /// The FK target type for `set_foreign_key` (the principal type).
    pub fk_target_type_id: TypeId,
    /// M2M only: the join table name. `None` for one-to-many.
    pub through_table: Option<String>,
    /// M2M only: FK column on the join table pointing to the principal.
    pub through_parent_fk_col: Option<String>,
    /// M2M only: FK column on the join table pointing to the child.
    pub through_child_fk_col: Option<String>,
}

/// Records a parent→child association for post-INSERT FK fixup.
///
/// For one-to-many: after the principal is inserted and its PK backfilled,
/// each child's FK field is set to the principal's PK via `set_foreign_key`.
///
/// For M2M: after both principal and child are inserted and their PKs
/// backfilled, a join-table row is inserted via direct SQL.
pub struct FixupLink {
    pub parent_type_id: TypeId,
    pub parent_entry_idx: usize,
    pub child_type_id: TypeId,
    pub child_entry_indices: Vec<usize>,
    pub fk_target_type_id: TypeId,
    /// M2M only: the join table name. `None` for one-to-many.
    pub through_table: Option<String>,
    pub through_parent_fk_col: Option<String>,
    pub through_child_fk_col: Option<String>,
}

/// Generates a batched `INSERT INTO through_table (parent_col, child_col)
/// VALUES (?, ?), (?, ?), ...` statement for M2M join rows.
pub fn m2m_insert_sql(
    table: &str,
    parent_col: &str,
    child_col: &str,
    row_count: usize,
) -> String {
    let placeholders: Vec<String> = (0..row_count).map(|_| "(?, ?)".to_string()).collect();
    format!(
        "INSERT INTO {} ({}, {}) VALUES {}",
        table,
        parent_col,
        child_col,
        placeholders.join(", ")
    )
}

// ---------------------------------------------------------------------------
// Cascade delete types
// ---------------------------------------------------------------------------

/// The action to take for untracked dependents when a principal is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeDeleteAction {
    /// `DELETE FROM child_table WHERE fk = ?`
    Delete,
    /// `UPDATE child_table SET fk = NULL WHERE fk = ?`
    SetNull,
}

/// A directive to delete or nullify untracked dependents of a Deleted
/// principal. Generated during the cascade delete drain loop and executed
/// as direct SQL at the start of the DELETE phase, before PK-based deletes.
pub struct CascadeDeleteDirective {
    /// The table to act on (child table for HasMany, join table for M2M).
    pub table: String,
    /// The FK column to filter on.
    pub fk_column: String,
    /// The principal's PK value.
    pub principal_pk: i64,
    /// The action to perform.
    pub action: CascadeDeleteAction,
}
