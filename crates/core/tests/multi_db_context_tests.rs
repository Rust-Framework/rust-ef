//! Integration tests for multi-database context key filtering.
//!
//! Verifies that:
//! 1. `#[context("key")]` on entity structs tags them for a keyed context.
//! 2. `#[entity(T, "key")]` on config impls applies only to matching context.
//! 3. `DbContext::from_options()` auto-discovers only entities matching the
//!    context's key.
//! 4. Default context (no key) only sees entities without `#[context(...)]`.

use rust_ef::prelude::*;
use rust_ef::{entity, EntityType};
use rust_ef_sqlite::DbContextOptionsBuilderExt;

// ---------------------------------------------------------------------------
// Default-context entities (no #[context] attribute)
// ---------------------------------------------------------------------------

#[derive(EntityType, Default, Clone, Debug, PartialEq)]
#[table("default_blogs")]
struct DefaultBlog {
    #[primary_key]
    #[auto_increment]
    id: i64,
    title: String,
}

#[derive(Default)]
struct DefaultBlogConfig;

#[entity(DefaultBlog)]
impl IEntityTypeConfiguration<DefaultBlog> for DefaultBlogConfig {
    fn configure(&self, e: &mut EntityTypeBuilder<'_, DefaultBlog>) {
        e.to_table("blogs_default_ctx");
    }
}

// ---------------------------------------------------------------------------
// "logs" keyed-context entity
// ---------------------------------------------------------------------------

#[derive(EntityType, Default, Clone, Debug, PartialEq)]
#[table("log_entries")]
#[context("logs")]
struct LogEntry {
    #[primary_key]
    #[auto_increment]
    id: i64,
    message: String,
}

#[derive(Default)]
struct LogEntryConfig;

#[entity(LogEntry, "logs")]
impl IEntityTypeConfiguration<LogEntry> for LogEntryConfig {
    fn configure(&self, e: &mut EntityTypeBuilder<'_, LogEntry>) {
        e.to_table("app_logs");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that `#[context("logs")]` causes the entity to be tagged with
/// `context_key = Some("logs")` in the inventory registration.
#[test]
fn test_context_key_registration_tags() {
    let regs: Vec<&rust_ef::registration::EntityRegistration> =
        inventory::iter::<rust_ef::registration::EntityRegistration>().collect();

    // DefaultBlog should have context_key = None
    let default_blog_reg = regs
        .iter()
        .find(|r| r.type_name.contains("DefaultBlog"))
        .expect("DefaultBlog should be registered");
    assert!(
        default_blog_reg.context_key.is_none(),
        "DefaultBlog should have context_key = None (default context)"
    );

    // LogEntry should have context_key = Some("logs")
    let log_entry_reg = regs
        .iter()
        .find(|r| r.type_name.contains("LogEntry"))
        .expect("LogEntry should be registered");
    assert_eq!(
        log_entry_reg.context_key,
        Some("logs"),
        "LogEntry should have context_key = Some(\"logs\")"
    );
}

/// Verify that `#[entity(T, "key")]` tags the config registration with
/// the matching context key.
#[test]
fn test_config_context_key_tags() {
    let configs: Vec<&rust_ef::registration::EntityConfigRegistration> =
        inventory::iter::<rust_ef::registration::EntityConfigRegistration>().collect();

    let default_blog_cfg = configs
        .iter()
        .find(|r| r.type_name.contains("DefaultBlog"))
        .expect("DefaultBlogConfig should be registered");
    assert!(
        default_blog_cfg.context_key.is_none(),
        "DefaultBlogConfig should have context_key = None"
    );

    let log_entry_cfg = configs
        .iter()
        .find(|r| r.type_name.contains("LogEntry"))
        .expect("LogEntryConfig should be registered");
    assert_eq!(
        log_entry_cfg.context_key,
        Some("logs"),
        "LogEntryConfig should have context_key = Some(\"logs\")"
    );
}

/// Verify that a default DbContext (no context_key) only discovers
/// DefaultBlog, NOT LogEntry.
#[test]
fn test_default_context_filters_out_keyed_entities() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite(":memory:");
    let options = builder.build();
    let ctx = DbContext::from_options(&options).expect("from_options should succeed");

    // DefaultBlog meta should be present (context_key = None matches)
    let has_default_blog = ctx.entity_metas_contains::<DefaultBlog>();
    assert!(
        has_default_blog,
        "Default context should discover DefaultBlog"
    );

    // LogEntry meta should NOT be present (context_key = Some("logs") doesn't match None)
    let has_log_entry = ctx.entity_metas_contains::<LogEntry>();
    assert!(
        !has_log_entry,
        "Default context should NOT discover LogEntry (it's tagged for \"logs\" context)"
    );
}

/// Verify that a keyed DbContext ("logs") only discovers LogEntry,
/// NOT DefaultBlog.
#[test]
fn test_keyed_context_filters_out_default_entities() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite(":memory:");
    builder.context_key("logs");
    let options = builder.build();
    let ctx = DbContext::from_options(&options).expect("from_options should succeed");

    // LogEntry meta should be present (context_key = Some("logs") matches)
    let has_log_entry = ctx.entity_metas_contains::<LogEntry>();
    assert!(has_log_entry, "\"logs\" context should discover LogEntry");

    // DefaultBlog meta should NOT be present (context_key = None doesn't match Some("logs"))
    let has_default_blog = ctx.entity_metas_contains::<DefaultBlog>();
    assert!(
        !has_default_blog,
        "\"logs\" context should NOT discover DefaultBlog (it's tagged for default context)"
    );
}

/// Verify that Fluent config overrides are applied only to the matching
/// context. DefaultBlogConfig renames the table to "blogs_default_ctx" —
/// this should be applied in the default context.
#[test]
fn test_default_context_applies_config_overrides() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite(":memory:");
    let options = builder.build();
    let ctx = DbContext::from_options(&options).expect("from_options should succeed");

    let metas = ctx.model_builder().build();
    let blog_meta = metas
        .iter()
        .find(|m| m.type_name.contains("DefaultBlog"))
        .expect("DefaultBlog meta should exist in model builder");

    assert_eq!(
        blog_meta.table_name, "blogs_default_ctx",
        "DefaultBlogConfig's to_table override should be applied in default context"
    );
}

/// Verify that LogEntryConfig's table rename to "app_logs" is applied
/// only in the "logs" context, not in the default context.
#[test]
fn test_keyed_context_applies_config_overrides() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite(":memory:");
    builder.context_key("logs");
    let options = builder.build();
    let ctx = DbContext::from_options(&options).expect("from_options should succeed");

    let metas = ctx.model_builder().build();
    let log_meta = metas
        .iter()
        .find(|m| m.type_name.contains("LogEntry"))
        .expect("LogEntry meta should exist in \"logs\" context model builder");

    assert_eq!(
        log_meta.table_name, "app_logs",
        "LogEntryConfig's to_table override should be applied in \"logs\" context"
    );
}
