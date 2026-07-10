//! Snapshot and migration data types.

/// Represents a single migration with up/down SQL scripts.
#[derive(Debug, Clone)]
pub struct Migration {
    pub id: String,
    pub description: String,
    pub up_sql: String,
    pub down_sql: String,
}

/// A snapshot of the entity model at a point in time.
/// Corresponds to EFCore's `ModelSnapshot`.
#[derive(Debug, Clone)]
pub struct ModelSnapshot {
    pub migration_id: String,
    pub entity_types: Vec<SnapshotEntityType>,
}

/// Serialized form of EntityTypeMeta for snapshot storage.
#[derive(Debug, Clone)]
pub struct SnapshotEntityType {
    pub type_name: String,
    pub table_name: String,
    pub columns: Vec<SnapshotColumn>,
}

/// Serialized form of PropertyMeta for snapshot storage.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnapshotColumn {
    pub field_name: String,
    pub column_name: String,
    pub type_name: String,
    pub is_primary_key: bool,
    pub is_required: bool,
    pub is_foreign_key: bool,
    pub max_length: Option<usize>,
    pub is_auto_increment: bool,
    /// Whether this column is backed by a database sequence (PostgreSQL).
    pub is_sequence: bool,
    /// The sequence name when `is_sequence` is true.
    pub sequence_name: Option<String>,
    /// Referenced table when `is_foreign_key` is true.
    pub fk_referenced_table: Option<String>,
    /// Referenced column when `is_foreign_key` is true.
    pub fk_referenced_column: Option<String>,
    /// Non-unique index on this column.
    pub has_index: bool,
    /// Unique constraint/index on this column.
    pub is_unique: bool,
    /// ON DELETE clause for the FK constraint (e.g. "CASCADE", "SET NULL").
    /// `None` means no explicit clause (DB default applies).
    pub fk_on_delete: Option<String>,
}
