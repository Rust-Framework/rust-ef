//! Atomic schema change representation — the output of model diffing.

use super::types::{
    SnapshotCheck, SnapshotColumn, SnapshotForeignKey, SnapshotIndex, SnapshotTableOptions,
};

/// Single atomic schema change detected during diffing.
#[derive(Debug, Clone)]
pub(crate) enum SchemaChange {
    CreateTable {
        table: String,
        columns: Vec<SnapshotColumn>,
        checks: Vec<SnapshotCheck>,
        table_options: SnapshotTableOptions,
        /// Composite foreign keys. Single-column keys live on `columns`.
        /// SQLite emits both inline (it cannot ALTER TABLE ADD CONSTRAINT).
        foreign_keys: Vec<SnapshotForeignKey>,
        /// Table comment.
        comment: Option<String>,
    },
    DropTable {
        table: String,
    },
    AddColumn {
        table: String,
        column: SnapshotColumn,
    },
    DropColumn {
        table: String,
        column_name: String,
    },
    AlterColumn {
        table: String,
        column_name: String,
        old: Box<SnapshotColumn>,
        new: Box<SnapshotColumn>,
    },
    AddForeignKey {
        table: String,
        column: String,
        referenced_table: String,
        referenced_column: String,
        on_delete: Option<String>,
    },
    DropForeignKey {
        table: String,
        column: String,
        referenced_table: String,
    },
    CreateIndex {
        table: String,
        column: String,
        is_unique: bool,
    },
    DropIndex {
        table: String,
        column: String,
        is_unique: bool,
    },
    /// Create a declared index (composite / partial / expression).
    CreateDeclaredIndex {
        table: String,
        index: SnapshotIndex,
    },
    /// Drop a declared index (definition retained so Down can recreate it).
    DropDeclaredIndex {
        table: String,
        index: SnapshotIndex,
    },
    /// Add a composite foreign key.
    AddCompositeForeignKey {
        table: String,
        foreign_key: SnapshotForeignKey,
    },
    /// Drop a composite foreign key (definition retained for Down).
    DropCompositeForeignKey {
        table: String,
        foreign_key: SnapshotForeignKey,
    },
    /// Add a table-level CHECK constraint.
    AddCheck {
        table: String,
        check: SnapshotCheck,
    },
    /// Drop a table-level CHECK constraint (definition retained for Down).
    DropCheck {
        table: String,
        check: SnapshotCheck,
    },
    /// Change MySQL table options (`ENGINE` / `CHARSET` / `COLLATE`).
    AlterTableOptions {
        table: String,
        options: SnapshotTableOptions,
    },
    /// Change the table comment (old value retained for Down).
    AlterTableComment {
        table: String,
        old_comment: Option<String>,
        new_comment: Option<String>,
    },
}
