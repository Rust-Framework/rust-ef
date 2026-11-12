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
#[derive(Debug, Clone, Default)]
pub struct SnapshotEntityType {
    pub type_name: String,
    pub table_name: String,
    pub columns: Vec<SnapshotColumn>,
    /// Declarative indexes (composite / unique / partial / expression).
    pub indexes: Vec<SnapshotIndex>,
    /// Composite foreign keys (single-column FKs live on the column).
    pub foreign_keys: Vec<SnapshotForeignKey>,
    /// Table-level CHECK constraints.
    pub checks: Vec<SnapshotCheck>,
    /// MySQL table options.
    pub table_options: SnapshotTableOptions,
    /// Maps to an existing view (no table DDL).
    pub is_view: bool,
    /// Keyless entity (no primary key).
    pub is_keyless: bool,
    /// Table comment.
    pub comment: Option<String>,
}

/// One key part of a snapshot index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotIndexColumn {
    /// Column name or raw SQL expression.
    pub value: String,
    /// Whether `value` is a raw expression.
    pub is_expression: bool,
}

/// A declared index in a model snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotIndex {
    /// Explicit or derived index name.
    pub name: String,
    /// Key parts, in order.
    pub columns: Vec<SnapshotIndexColumn>,
    /// UNIQUE index.
    pub is_unique: bool,
    /// Rendered partial-index predicate (dialect-specific literals).
    pub filter: Option<String>,
    /// Non-key included columns (`INCLUDE`).
    pub include: Vec<String>,
}

/// A composite foreign key in a model snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotForeignKey {
    /// Explicit or derived constraint name.
    pub name: String,
    /// Dependent columns.
    pub columns: Vec<String>,
    /// Principal table.
    pub referenced_table: String,
    /// Principal columns.
    pub referenced_columns: Vec<String>,
    /// `ON DELETE` clause.
    pub on_delete: Option<String>,
}

/// A CHECK constraint in a model snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotCheck {
    pub name: String,
    pub expression: String,
}

/// MySQL table options in a model snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SnapshotTableOptions {
    pub engine: Option<String>,
    pub charset: Option<String>,
    pub collate: Option<String>,
}

impl SnapshotTableOptions {
    pub fn is_empty(&self) -> bool {
        self.engine.is_none() && self.charset.is_none() && self.collate.is_none()
    }
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
    /// Raw SQL DEFAULT expression (`None` = no default).
    pub default_value: Option<String>,
    /// Explicit database type (`HasColumnType`), overrides dialect mapping.
    pub column_type: Option<String>,
    /// Decimal/temporal precision.
    pub precision: Option<u32>,
    /// Decimal scale.
    pub scale: Option<u32>,
    /// Fixed-length string type (`CHAR(n)`).
    pub is_fixed_length: bool,
    /// Column collation.
    pub collation: Option<String>,
    /// Column comment.
    pub comment: Option<String>,
    /// Computed/generated column expression.
    pub computed_sql: Option<String>,
    /// Persisted (`STORED`) computed column.
    pub computed_stored: bool,
}
