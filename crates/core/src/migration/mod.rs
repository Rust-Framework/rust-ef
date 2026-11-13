//! Migration engine — model snapshot diffing, migration generation, and history tracking.
//!
//! Corresponds to EFCore's migration system. Implements full model diffing:
//!   - Detect added/removed tables
//!   - Detect added/removed/altered columns
//!   - Generate Up/Down SQL with dialect-specific type mappings
//!   - Maintain `__ef_migrations_history` tracking table
//!
//! Runtime upgrades (installed databases, model as the source of truth) use
//! [`MigrationEngine::diff`] against an introspected [`crate::schema::DbTable`]
//! list and [`MigrationEngine::apply_diff`], see the `live_diff` module.

mod ddl_expr;
mod dialect;
mod diff;
mod diff_apply;
mod engine;
mod engine_ops;
mod engine_sql;
mod history;
mod live_diff;
mod schema_change;
mod snapshot;
mod snapshot_io;
mod types;

pub use dialect::MigrationDialect;
pub use diff_apply::DiffApplyResult;
pub use engine::MigrationEngine;
pub use history::{
    create_migration_history_table_sql, validate_migration_id, MigrationHistoryEntry,
    MigrationStore, MIGRATION_HISTORY_TABLE, PRODUCT_VERSION,
};
pub use live_diff::{DiffWarning, MigrationDiff, TableRebuild, WarningKind, WarningSeverity};
pub use snapshot::{build_registered_snapshot, save_registered_snapshot, write_snapshot};
pub use snapshot_io::parse_model_snapshot_json;
pub use types::{
    Migration, ModelSnapshot, SnapshotCheck, SnapshotColumn, SnapshotEntityType,
    SnapshotForeignKey, SnapshotIndex, SnapshotIndexColumn, SnapshotTableOptions,
};
