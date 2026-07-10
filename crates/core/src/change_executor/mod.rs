//! Change executor — generates and executes SQL for entity state changes.
//!
//! The `ChangeExecutor` takes a collection of tracked entities grouped by
//! state (Added/Modified/Deleted), generates the appropriate parameterized
//! DML, and executes it against the database via the provider.
//!
//! INSERT/UPSERT methods live in `executor.rs`; UPDATE/DELETE methods (with
//! batched + per-row fallback) in `executor_dml.rs`; standalone SQL generation
//! helpers in `sql_gen.rs`.

mod executor;
mod executor_dml;
mod sql_gen;

pub use executor::ChangeExecutor;
pub use sql_gen::{
    collect_delete_params, collect_insert_params, collect_update_params, generate_delete_sql,
    generate_insert_sql, generate_update_sql,
};
