//! Tests for the process-level `MetadataCache` on `DbContextOptions`.
//!
//! Verifies three properties of the cached metadata path:
//! 1. Two `DbContext` instances from the same options both see the correct
//!    entity metadata (functional correctness of the cache path).
//! 2. Per-instance `ModelBuilder` mutations (`has_query_filter`) on one
//!    context do not leak into another context from the same options.
//! 3. The `discover_entities()` no-op (retained for backward compatibility)
//!    does not interfere with the pre-populated metadata.

use rust_ef::prelude::*;
use rust_ef::{entity, EntityType};
use rust_ef_sqlite::DbContextOptionsBuilderExt;
use std::any::TypeId;

// ---------------------------------------------------------------------------
// Test entity (default context)
// ---------------------------------------------------------------------------

#[derive(EntityType, Default, Clone, Debug, PartialEq)]
#[table("cache_blogs")]
struct CacheBlog {
    #[primary_key]
    #[auto_increment]
    id: i64,
    rating: i32,
}

#[derive(Default)]
struct CacheBlogConfig;

#[entity(CacheBlog)]
impl IEntityTypeConfiguration<CacheBlog> for CacheBlogConfig {
    fn configure(&self, e: &mut EntityTypeBuilder<'_, CacheBlog>) {
        e.to_table("cache_blogs_v2");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two `DbContext` instances created from the same `DbContextOptions` should
/// both have `CacheBlog` metadata populated (via the shared `MetadataCache`).
///
/// This verifies that `from_options()` → `MetadataCache::get_or_build()` →
/// `built.entity_metas.clone()` correctly populates `DbContext.entity_metas`.
#[test]
fn test_cache_populates_metadata_correctly() {
    let mut options_builder = DbContextOptionsBuilder::new();
    options_builder.use_sqlite_in_memory();
    let options = options_builder.build();

    let ctx1 = DbContext::from_options(&options).expect("ctx1 from_options");
    let ctx2 = DbContext::from_options(&options).expect("ctx2 from_options");

    // Both contexts should see CacheBlog metadata, populated from the cache.
    assert!(
        ctx1.entity_metas_contains::<CacheBlog>(),
        "ctx1 should have CacheBlog metadata from cache"
    );
    assert!(
        ctx2.entity_metas_contains::<CacheBlog>(),
        "ctx2 should have CacheBlog metadata from cache (shared options)"
    );

    // The Fluent configuration (to_table "cache_blogs_v2") should also be
    // applied via the cached configs. Verify by building the model and
    // checking the table name override.
    let metas1 = ctx1.model_builder().build();
    let metas2 = ctx2.model_builder().build();
    let blog_meta1 = metas1
        .iter()
        .find(|m| m.type_name.contains("CacheBlog"))
        .expect("CacheBlog meta in ctx1 build output");
    let blog_meta2 = metas2
        .iter()
        .find(|m| m.type_name.contains("CacheBlog"))
        .expect("CacheBlog meta in ctx2 build output");
    assert_eq!(
        blog_meta1.table_name, "cache_blogs_v2",
        "ctx1 should have Fluent override from cached config"
    );
    assert_eq!(
        blog_meta2.table_name, "cache_blogs_v2",
        "ctx2 should have same Fluent override from cached config"
    );
}

/// Per-instance `ModelBuilder` mutation on one `DbContext` must not affect
/// another `DbContext` created from the same options.
///
/// `ctx1.model().has_query_filter::<CacheBlog>(...)` writes to ctx1's own
/// `ModelBuilder.configs`, not to the shared `BuiltMetadata` cache. ctx2's
/// `ModelBuilder` (cloned from the same cache) should not see the filter.
#[test]
fn test_per_instance_model_mutation_isolation() {
    let mut options_builder = DbContextOptionsBuilder::new();
    options_builder.use_sqlite_in_memory();
    let options = options_builder.build();

    let mut ctx1 = DbContext::from_options(&options).expect("ctx1 from_options");
    let ctx2 = DbContext::from_options(&options).expect("ctx2 from_options");

    // ctx1 adds a query filter — this writes to ctx1's per-instance ModelBuilder.
    ctx1
        .model()
        .has_query_filter::<CacheBlog>(linq!(filter |b: CacheBlog| b.rating > 5));

    let blog_type_id = TypeId::of::<CacheBlog>();

    // ctx1 should have the filter.
    assert!(
        ctx1.model_builder().get_query_filter(&blog_type_id).is_some(),
        "ctx1 should have the query filter it just registered"
    );

    // ctx2 should NOT have the filter — per-instance isolation.
    assert!(
        ctx2.model_builder().get_query_filter(&blog_type_id).is_none(),
        "ctx2 should not see ctx1's per-instance query filter (cache must not be mutated)"
    );
}

/// The `discover_entities()` method is now a no-op (metadata is pre-populated
/// from cache in `from_options()`). Calling it explicitly after
/// `from_options()` should not clear or corrupt the metadata.
#[test]
fn test_discover_entities_noop_is_safe() {
    let mut options_builder = DbContextOptionsBuilder::new();
    options_builder.use_sqlite_in_memory();
    let options = options_builder.build();

    let mut ctx = DbContext::from_options(&options).expect("ctx from_options");

    // Metadata is already populated from cache.
    assert!(ctx.entity_metas_contains::<CacheBlog>());

    // Explicit discover_entities() call — should be a no-op, not corrupt anything.
    ctx.discover_entities().expect("discover_entities no-op");

    // Metadata should still be present.
    assert!(
        ctx.entity_metas_contains::<CacheBlog>(),
        "discover_entities() no-op must not clear pre-populated metadata"
    );
}
