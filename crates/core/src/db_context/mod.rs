//! DbContext — the session / unit-of-work layer.
//!
//! See `context.rs` for the `DbContext` struct and non-save methods.
//! Save pipeline lives in `save_pipeline.rs`; cascade phases in `save_phases.rs`.

mod context;
mod options;
mod save_phases;
mod save_pipeline;
mod set_ops;

pub use context::DbContext;
pub use options::{DbContextOptions, DbContextOptionsBuilder};
pub use save_phases::{
    delete_deleted_phase, insert_added_phase, save_one_set, update_modified_phase,
    upsert_added_phase, SaveChangesResult,
};
pub(crate) use set_ops::{resolve_delete_behavior, ErasedSetOps, SetOps};
