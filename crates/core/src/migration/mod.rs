//! Migration engine — model snapshot diffing, migration generation, and history tracking.
//!
//! Corresponds to EFCore's migration system. Implements full model diffing:
//!   - Detect added/removed tables
//!   - Detect added/removed/altered columns
//!   - Generate Up/Down SQL with dialect-specific type mappings
//!   - Maintain `__ef_migrations_history` tracking table

mod dialect;
mod diff;
mod engine;
mod engine_ops;
mod engine_sql;
mod history;
mod schema_change;
mod snapshot_io;
mod types;

pub use dialect::MigrationDialect;
pub use engine::MigrationEngine;
pub use history::{
    create_migration_history_table_sql, MigrationHistoryEntry, MigrationStore,
    MIGRATION_HISTORY_TABLE, PRODUCT_VERSION,
};
pub use snapshot_io::parse_model_snapshot_json;
pub use types::{Migration, ModelSnapshot, SnapshotColumn, SnapshotEntityType};
