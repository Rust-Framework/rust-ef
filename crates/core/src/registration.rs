//! Compile-time entity registration via `inventory`.
//!
//! The `#[derive(EntityType)]` macro emits an `inventory::submit!` for each
//! entity type, registering a type-erased [`EntityRegistration`]. The
//! `DbContext` discovers these at runtime via
//! `inventory::iter::<EntityRegistration>()` (see `DbContext::discover_entities`).
//!
//! Similarly, `#[entity_config(T)]` applied to `impl IEntityTypeConfiguration<T>`
//! blocks emits an [`EntityConfigRegistration`], whose `apply_fn` is invoked by
//! `DbContext::discover_entities()` to apply Fluent API overrides to the
//! `ModelBuilder`.

use crate::metadata::EntityTypeMeta;
use crate::model_builder::ModelBuilder;
use std::any::TypeId;

/// Type-erased registration for an entity type.
///
/// Emitted automatically by `#[derive(EntityType)]`. Collected at link time
/// via `inventory::collect!`, so every entity type compiled into the final
/// binary is visible to `DbContext::discover_entities()`.
#[derive(Debug)]
pub struct EntityRegistration {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub meta_fn: fn() -> EntityTypeMeta,
}

impl EntityRegistration {
    pub fn meta(&self) -> EntityTypeMeta {
        (self.meta_fn)()
    }
}

inventory::collect!(EntityRegistration);

/// Type-erased registration for an `IEntityTypeConfiguration<T>` impl block.
///
/// Emitted by the `#[entity_config(T)]` attribute macro. The `apply_fn`
/// instantiates the configuration via `Default::default()` and invokes
/// `IEntityTypeConfiguration::configure(&mut EntityTypeBuilder)` on a
/// freshly-borrowed `ModelBuilder`.
#[derive(Clone, Copy)]
pub struct EntityConfigRegistration {
    pub type_id: TypeId,
    pub type_name: &'static str,
    pub apply_fn: fn(&mut ModelBuilder),
}

impl std::fmt::Debug for EntityConfigRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityConfigRegistration")
            .field("type_id", &self.type_id)
            .field("type_name", &self.type_name)
            .finish()
    }
}

inventory::collect!(EntityConfigRegistration);
