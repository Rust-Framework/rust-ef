//! Process-level cache of entity metadata, keyed by `context_key`.
//!
//! `DbContext::from_options()` calls `MetadataCache::get_or_build()` instead of
//! re-iterating `inventory::iter` on every request. The first call for a given
//! `context_key` runs all `IEntityTypeConfiguration::configure()` callbacks and
//! constructs all `EntityTypeMeta` instances; subsequent calls receive an
//! `Arc<BuiltMetadata>` clone.
//!
//! The cache lives on `DbContextOptions`, which is `Arc`-shared across all
//! `DbContext` instances created from the same `add_dbcontext` registration
//! (see `di.rs`). This makes the cache naturally singleton-per-registration.
//!
//! Per-instance `ModelBuilder` mutations (`has_query_filter`, etc.) after
//! `from_options()` only affect that `DbContext`'s own `ModelBuilder` — the
//! cache is never mutated post-build.

use crate::metadata::EntityTypeMeta;
use crate::model_builder::{EntityConfig, ModelBuilder};
use crate::registration::{EntityConfigRegistration, EntityRegistration};
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The output of running `discover_entities()` for a given `context_key`.
///
/// Cached on `DbContextOptions` so all `DbContext` instances sharing the same
/// options share the same parsed metadata — `inventory::iter` and
/// `IEntityTypeConfiguration::configure()` run once per `context_key`, ever.
#[derive(Clone)]
pub(crate) struct BuiltMetadata {
    /// `TypeId -> EntityTypeMeta` map, cloned into `DbContext.entity_metas`.
    pub entity_metas: HashMap<TypeId, EntityTypeMeta>,
    /// `Vec<EntityTypeMeta>`, cloned into `ModelBuilder.entity_metas`.
    pub model_metas: Vec<EntityTypeMeta>,
    /// Base configs from `IEntityTypeConfiguration::configure()`, cloned into
    /// `ModelBuilder.configs`. Per-instance query-filter mutations layer on top.
    pub configs: HashMap<TypeId, EntityConfig>,
}

/// Process-wide cache of `BuiltMetadata` keyed by `context_key`.
///
/// Stored on `DbContextOptions` (which is `Arc`-shared across all `DbContext`
/// instances created from the same `add_dbcontext` registration).
pub(crate) struct MetadataCache {
    by_key: RwLock<HashMap<Option<String>, Arc<BuiltMetadata>>>,
}

impl MetadataCache {
    pub fn new() -> Self {
        Self {
            by_key: RwLock::new(HashMap::new()),
        }
    }

    /// Returns the `BuiltMetadata` for the given `context_key`, building it
    /// on first access.
    ///
    /// Read path uses a shared `read()` guard for cache hits (the common case
    /// after warm-up). On miss, the read guard is dropped and a `write()` guard
    /// is acquired to build + insert, with a double-check to avoid redundant
    /// builds when multiple threads race for the same key.
    ///
    /// If the lock is poisoned (a previous holder panicked), the cache is
    /// cleared and rebuilt rather than panicking. A poison indicates the
    /// prior build was interrupted, so cached entries may be incomplete.
    /// Clearing forces a fresh `build()` on the next access.
    pub fn get_or_build(&self, context_key: Option<&str>) -> Arc<BuiltMetadata> {
        let key = context_key.map(|s| s.to_string());

        // Fast path: shared read lock for cache lookup. On poison, fall
        // through to the write path which can clear + rebuild.
        if let Ok(cache) = self.by_key.read() {
            if let Some(built) = cache.get(&key) {
                return Arc::clone(built);
            }
        }

        // Slow path: exclusive write lock for build + insert.
        let mut cache = match self.by_key.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                guard.clear();
                guard
            }
        };
        // Double-check: another thread may have built and inserted while we
        // were waiting for the write lock.
        if let Some(existing) = cache.get(&key) {
            return Arc::clone(existing);
        }
        let built = Arc::new(Self::build(context_key));
        cache.insert(key, Arc::clone(&built));
        built
    }

    /// Runs the `discover_entities()` logic: iterates `inventory::iter` for
    /// both `EntityConfigRegistration` and `EntityRegistration`, filtered by
    /// `context_key`, and snapshots the result.
    ///
    /// Mirrors `DbContext::discover_entities()` (db_context.rs) — keep the
    /// two in sync if the inventory iteration order or filtering changes.
    fn build(context_key: Option<&str>) -> BuiltMetadata {
        let mut model_builder = ModelBuilder::new();

        // 1. Apply Fluent configurations matching this context's key.
        //    `apply_fn` invokes `IEntityTypeConfiguration::configure()`, which
        //    populates `model_builder.configs` (table_name, property overrides,
        //    query filters, seed rows, etc.).
        for reg in inventory::iter::<EntityConfigRegistration> {
            if reg.context_key == context_key {
                (reg.apply_fn)(&mut model_builder);
            }
        }

        // 2. Register entity metadata for entities matching this context's key.
        //    `reg.meta()` constructs a fresh `EntityTypeMeta` via the
        //    `#[derive(EntityType)]`-emitted `meta_fn`.
        let mut entity_metas: HashMap<TypeId, EntityTypeMeta> = HashMap::new();
        for reg in inventory::iter::<EntityRegistration> {
            if reg.context_key == context_key {
                let meta = reg.meta();
                let type_id = reg.type_id;
                entity_metas.entry(type_id).or_insert_with(|| meta.clone());
                if !model_builder.has_entity(type_id) {
                    model_builder.register_entity_meta(meta);
                }
            }
        }

        // 3. Snapshot the model_builder state into the cache.
        let model_metas: Vec<EntityTypeMeta> = model_builder.entity_metas_vec().to_vec();
        let configs: HashMap<TypeId, EntityConfig> = model_builder.configs().clone();

        BuiltMetadata {
            entity_metas,
            model_metas,
            configs,
        }
    }
}

impl Default for MetadataCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that a poisoned `MetadataCache` recovers gracefully instead of
    /// panicking.
    ///
    /// A poison occurs when a thread panics while holding the lock. Previously
    /// `get_or_build` called `.expect("MetadataCache poisoned")`, which would
    /// propagate the panic to every subsequent caller — a single failure
    /// permanently broke the whole process. The fix clears the cache on poison
    /// and rebuilds from inventory.
    #[test]
    fn test_poison_recovery() {
        let cache = MetadataCache::new();

        // First call builds and caches the metadata for context_key=None.
        let built1 = cache.get_or_build(None);
        let first_entity_count = built1.entity_metas.len();
        let first_model_count = built1.model_metas.len();

        // Intentionally poison the RwLock by panicking while holding the
        // write lock. catch_unwind isolates the panic so the test continues.
        let _ = std::panic::catch_unwind(|| {
            let _guard = cache.by_key.write().unwrap();
            panic!("intentional poison for test");
        });

        // The RwLock is now poisoned. get_or_build must recover (clear +
        // rebuild) rather than panic. This is the core assertion — if the
        // fix regresses, this call panics and the test fails.
        let built2 = cache.get_or_build(None);

        // The rebuilt metadata should have the same shape as the first build
        // (same number of entities and models), proving the rebuild is
        // deterministic and complete.
        assert_eq!(
            first_entity_count,
            built2.entity_metas.len(),
            "rebuilt metadata should have the same entity count as the first build"
        );
        assert_eq!(
            first_model_count,
            built2.model_metas.len(),
            "rebuilt metadata should have the same model count as the first build"
        );
    }

    /// A second call with the same context_key should return the cached
    /// `Arc<BuiltMetadata>` (cheap clone), not rebuild.
    #[test]
    fn test_cache_hit_returns_same_arc() {
        let cache = MetadataCache::new();
        let built1 = cache.get_or_build(None);
        let built2 = cache.get_or_build(None);
        // Arc::ptr_eq checks pointer identity — a cache hit returns a clone
        // of the same Arc, not a freshly built one.
        assert!(
            Arc::ptr_eq(&built1, &built2),
            "second call with same key should return the same cached Arc"
        );
    }

    /// Different context_keys should produce independent `BuiltMetadata`
    /// entries (no cross-key leakage).
    #[test]
    fn test_different_context_keys_are_isolated() {
        let cache = MetadataCache::new();
        let built_default = cache.get_or_build(None);
        let built_keyed = cache.get_or_build(Some("other"));
        assert!(
            !Arc::ptr_eq(&built_default, &built_keyed),
            "different context_keys should produce independent BuiltMetadata"
        );
    }
}
