//! Tests for `ModelBuilder` OnceLock-based caching.
//!
//! Verifies:
//! 1. First `build()` populates the cache; subsequent calls hit the cache.
//! 2. First `filters_by_table()` populates the cache; subsequent calls hit.
//! 3. Any mutating Fluent API call (`to_table`, `has_query_filter`, property
//!    overrides, `register_entity_meta`) invalidates both caches so the next
//!    read observes the new configuration.
//! 4. The `Arc` returned by `filters_by_table()` is cheaply cloneable and
//!    reflects the latest filter set after invalidation.

use rust_ef::model_builder::ModelBuilder;
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::{BoolExpr, FilterCondition};

#[derive(Debug, Clone, EntityType)]
#[table("cache_blogs")]
struct CacheBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    url: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("cache_posts")]
struct CachePost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(CacheBlog)]
    blog_id: i32,
    #[required]
    tenant_id: i32,
}

fn tenant_filter(value: i32) -> BoolExpr {
    BoolExpr::Filter(FilterCondition::with_values(
        "tenant_id",
        "=",
        vec![DbValue::I32(value)],
    ))
}

#[test]
fn build_populates_cache_and_reuses() {
    let mut model = ModelBuilder::new();
    model.entity::<CacheBlog>();
    model.entity::<CachePost>();

    assert!(
        !model.build_cache_populated(),
        "cache cold before first build"
    );

    let first = model.build();
    assert!(
        model.build_cache_populated(),
        "cache warm after first build"
    );

    // Second call must return equal content (cache hit, not rebuild).
    let second = model.build();
    assert_eq!(first.len(), second.len());
    assert_eq!(
        first
            .iter()
            .map(|m| m.table_name.as_ref())
            .collect::<Vec<_>>(),
        second
            .iter()
            .map(|m| m.table_name.as_ref())
            .collect::<Vec<_>>(),
    );
}

#[test]
fn filters_by_table_populates_cache_and_returns_arc() {
    let mut model = ModelBuilder::new();
    model.entity::<CachePost>();
    model.has_query_filter::<CachePost>(tenant_filter(1));

    assert!(
        !model.filter_cache_populated(),
        "filter cache cold before first call"
    );

    let first = model.filters_by_table();
    assert!(
        model.filter_cache_populated(),
        "filter cache warm after first call"
    );
    assert_eq!(first.len(), 1, "one filter registered");
    assert!(
        first.contains_key("cache_posts"),
        "keyed by compile-time table name"
    );

    // Second call returns an equal Arc (cheap clone, same content).
    let second = model.filters_by_table();
    assert_eq!(first.len(), second.len());
}

#[test]
fn to_table_invalidates_build_cache() {
    let mut model = ModelBuilder::new();
    model.entity::<CacheBlog>();
    let _ = model.build();
    assert!(model.build_cache_populated(), "cache warm after build");

    // Fluent API override renames the table -> must invalidate.
    model.entity::<CacheBlog>().to_table("renamed_blogs");
    assert!(
        !model.build_cache_populated(),
        "cache invalidated after to_table"
    );

    let metas = model.build();
    let blog_meta = metas
        .iter()
        .find(|m| m.type_id == std::any::TypeId::of::<CacheBlog>())
        .expect("blog meta present");
    assert_eq!(blog_meta.table_name.as_ref(), "renamed_blogs");
    assert!(
        model.build_cache_populated(),
        "cache repopulated after rebuild"
    );
}

#[test]
fn has_query_filter_invalidates_filter_cache() {
    let mut model = ModelBuilder::new();
    model.entity::<CachePost>();

    // No filter initially.
    let empty = model.filters_by_table();
    assert!(empty.is_empty());
    assert!(model.filter_cache_populated(), "filter cache warm");

    // Registering a filter must invalidate so the next read sees it.
    model.has_query_filter::<CachePost>(tenant_filter(7));
    assert!(
        !model.filter_cache_populated(),
        "filter cache invalidated after has_query_filter"
    );

    let filters = model.filters_by_table();
    assert_eq!(filters.len(), 1, "filter visible after invalidation");
    assert!(filters.contains_key("cache_posts"));
}

#[test]
fn property_override_invalidates_build_cache() {
    let mut model = ModelBuilder::new();
    model.entity::<CachePost>();
    let _ = model.build();
    assert!(model.build_cache_populated());

    // Property override via PropertyBuilder -> override_entry invalidates.
    model
        .entity::<CachePost>()
        .property_named("title")
        .has_max_length(200);

    assert!(
        !model.build_cache_populated(),
        "cache invalidated by property override"
    );

    let metas = model.build();
    let post_meta = metas
        .iter()
        .find(|m| m.type_id == std::any::TypeId::of::<CachePost>())
        .expect("post meta present");
    let title_prop = post_meta
        .properties
        .iter()
        .find(|p| p.field_name.as_ref() == "title")
        .expect("title property present");
    assert_eq!(title_prop.max_length, Some(200));
}

#[test]
fn register_entity_meta_invalidates_caches() {
    let mut model = ModelBuilder::new();
    model.entity::<CacheBlog>();
    let _ = model.build();
    let _ = model.filters_by_table();
    assert!(model.build_cache_populated());
    assert!(model.filter_cache_populated());

    // Direct meta registration (used by discover_entities) must invalidate.
    model.register_entity_meta(CachePost::entity_meta());
    assert!(
        !model.build_cache_populated(),
        "build cache invalidated by register_entity_meta"
    );
    assert!(
        !model.filter_cache_populated(),
        "filter cache invalidated by register_entity_meta"
    );

    let metas = model.build();
    assert_eq!(metas.len(), 2, "both entities present after rebuild");
}

#[test]
fn filters_keyed_by_compile_time_table_name_not_override() {
    // Fluent `to_table` renames the effective table, but query filters must
    // remain keyed by the compile-time name so navigation SQL (which uses the
    // compile-time name) can look them up.
    let mut model = ModelBuilder::new();
    model.entity::<CachePost>().to_table("posts_renamed");
    model.has_query_filter::<CachePost>(tenant_filter(1));

    let filters = model.filters_by_table();
    assert!(
        filters.contains_key("cache_posts"),
        "filter keyed by compile-time name, not Fluent override"
    );
    assert!(
        !filters.contains_key("posts_renamed"),
        "Fluent override name must not be used as filter key"
    );
}
