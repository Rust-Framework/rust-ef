//! Snapshot helpers for the automated upgrade workflow.
//!
//! `rust-ef migration add` needs a *current* model snapshot file to diff
//! against the baseline stored in the migrations directory. Because Rust has no
//! runtime reflection, the CLI cannot read the application's registered
//! entities; the application must render the snapshot itself (this module) and
//! then invoke the CLI.

use super::dialect::MigrationDialect;
use super::engine::MigrationEngine;
use super::types::ModelSnapshot;
use crate::error::{EFError, EFResult};
use crate::metadata_cache::MetadataCache;
use crate::model_builder::ModelBuilder;
use std::path::Path;

/// Builds a [`ModelSnapshot`] from all inventory-registered entities and
/// `#[entity(T)]` configurations for `context_key`.
///
/// Call this from a small binary/test in the application (e.g. a `snapshot`
/// subcommand), write it with [`write_snapshot`], then run
/// `rust-ef migration add --snapshot <file> --dir Migrations`.
pub fn build_registered_snapshot(
    context_key: Option<&str>,
    dialect: MigrationDialect,
) -> EFResult<ModelSnapshot> {
    let built = MetadataCache::build(context_key);
    let builder = ModelBuilder::from_built(&built);
    let metas = builder.build();
    if metas.is_empty() {
        return Err(EFError::configuration(
            "No entity types registered. Derive EntityType on the application entities \
             (and apply #[entity(T)] configurations) before building a snapshot.",
        ));
    }
    Ok(MigrationEngine::new(dialect).create_snapshot("__snapshot__", &metas))
}

/// Writes a snapshot to an explicit path (pretty JSON).
pub fn write_snapshot(path: impl AsRef<Path>, snapshot: &ModelSnapshot) -> EFResult<()> {
    let json = super::snapshot_io::snapshot_to_json(snapshot);
    if let Some(parent) = path.as_ref().parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                EFError::migration(format!("Cannot create {}: {e}", parent.display()))
            })?;
        }
    }
    std::fs::write(path.as_ref(), json)
        .map_err(|e| EFError::migration(format!("Cannot write snapshot: {e}")))
}

/// Builds the registered snapshot and writes it to `path`.
pub fn save_registered_snapshot(
    path: impl AsRef<Path>,
    context_key: Option<&str>,
    dialect: MigrationDialect,
) -> EFResult<ModelSnapshot> {
    let snapshot = build_registered_snapshot(context_key, dialect)?;
    write_snapshot(path, &snapshot)?;
    Ok(snapshot)
}
