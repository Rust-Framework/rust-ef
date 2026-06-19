use crate::entry::IServiceResolver;
use crate::lifetime::ServiceLifetime;
use std::any::{Any, TypeId};
use std::sync::Arc;

/// A runtime service registration collected via `inventory::submit!`.
/// Each `#[rust_dicore::inject(...)]` on a struct generates one submission.
///
/// `type_name_fn` is a function pointer (not a string) to avoid requiring
/// `std::any::type_name::<T>()` in a const context (tracking issue #63084).
/// The function is called at runtime by `from_injected()`.
pub struct ServiceRegistration {
    pub lifetime: ServiceLifetime,
    pub type_id: TypeId,
    pub type_name_fn: fn() -> &'static str,
    pub factory: fn(&dyn IServiceResolver) -> Arc<dyn Any + Send + Sync>,
}

// Safety: fn pointers + enum + TypeId are all Send + Sync + 'static
inventory::collect!(ServiceRegistration);
