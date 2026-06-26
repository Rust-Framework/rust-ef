//! Application `DbContext` using inventory-based auto-registration.
//!
//! Demonstrates the v0.5.1+ pattern:
//! 1. `#[derive(EntityType)]` automatically registers entities with `inventory`
//! 2. `#[entity_config(Blog)]` registers `BlogConfig` configuration
//! 3. `ctx.discover_entities()` populates both STORE A (entity_metas) and
//!    STORE B (model_builder) from the global registry
//! 4. `ctx.ensure_created()` applies all `#[entity_config]` overrides via
//!    `model_builder.build()`, then creates the schema

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::error::EFResult;
use rust_ef_sqlite::DbContextOptionsBuilderExt;

/// Creates an in-memory SQLite `DbContext` with auto-discovered schema.
///
/// The schema is built from:
/// - All `#[derive(EntityType)]` types in this crate (Blog, Post)
/// - All `#[entity_config(T)]` configurations (BlogConfig renames the table
///   to `blogs_renamed` and the `url` column to `blog_url`)
pub async fn create_blog_context() -> EFResult<DbContext> {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let mut ctx = DbContext::from_options(&builder.build())?;

    ctx.discover_entities()?;
    ctx.ensure_created().await?;

    Ok(ctx)
}
