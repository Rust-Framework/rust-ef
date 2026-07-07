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
use std::sync::{Arc, Mutex};

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
    by_key: Mutex<HashMap<Option<String>, Arc<BuiltMetadata>>>,
}

impl MetadataCache {
    pub fn new() -> Self {
        Self {
            by_key: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the `BuiltMetadata` for the given `context_key`, building it
    /// on first access.
    ///
    /// Lock is held only during lookup/insertion. After return, the
    /// `Arc<BuiltMetadata>` is lock-free.
    pub fn get_or_build(&self, context_key: Option<&str>) -> Arc<BuiltMetadata> {
        let key = context_key.map(|s| s.to_string());
        let mut cache = self.by_key.lock().expect("MetadataCache poisoned");
        if let Some(built) = cache.get(&key) {
            return Arc::clone(built);
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
