//! Atomic schema change representation — the output of model diffing.

use super::types::SnapshotColumn;

/// Single atomic schema change detected during diffing.
#[derive(Debug, Clone)]
pub(crate) enum SchemaChange {
    CreateTable {
        table: String,
        columns: Vec<SnapshotColumn>,
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
}
