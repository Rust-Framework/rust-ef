//! Bridge implementations that adapt RDI types to the `sdk::ServiceLocator` trait.
//!
//! The `ServiceLocatorBridge` combines:
//! 1. RDI provider (compile-time registered services, type_name-based)
//! 2. Named service registry (string-based, for cross-DLL access) �?
//!    stored directly in `ServiceProvider::named`.

use std::any::Any;
use std::sync::Arc;

use crate::service_locator::{INamedRegistrar, IServiceLocator};

use crate::entry::IServiceResolver;
use crate::provider::ServiceProvider;
use crate::wrapper::ServiceProviderWrapper;

/// Enum that wraps either a root `ServiceProvider` or a `ServiceProviderWrapper`
/// so both can be used interchangeably behind `IServiceResolver`.
pub enum RdiProvider {
    Root(Arc<ServiceProvider>),
    Wrapped(Arc<ServiceProviderWrapper>),
}

impl IServiceResolver for RdiProvider {
    fn get_any(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        match self {
            RdiProvider::Root(p) => IServiceResolver::get_any(p.as_ref(), key),
            RdiProvider::Wrapped(w) => IServiceResolver::get_any(w.as_ref(), key),
        }
    }

    fn get_keyed_any(&self, key: &str, variant: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        match self {
            RdiProvider::Root(p) => IServiceResolver::get_keyed_any(p.as_ref(), key, variant),
            RdiProvider::Wrapped(w) => IServiceResolver::get_keyed_any(w.as_ref(), key, variant),
        }
    }
}

impl RdiProvider {
    /// Non-generic named resolution for trait-object dispatch.
    pub fn get_named_any(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        match self {
            RdiProvider::Root(p) => p.get_named_any(name),
            RdiProvider::Wrapped(w) => w.get_named_any(name),
        }
    }

    /// Register a named service into the active (child) provider.
    pub fn register_named_any(&self, name: &str, service: Arc<dyn Any + Send + Sync>) {
        match self {
            RdiProvider::Root(p) => p.rdi_register_named(name, service),
            RdiProvider::Wrapped(w) => w.rdi_register_named(name, service),
        };
    }

    /// Remove a named service from the active (child) provider.
    pub fn remove_named(&self, name: &str) {
        match self {
            RdiProvider::Root(p) => {
                p.rdi_remove_named(name);
            }
            RdiProvider::Wrapped(w) => {
                w.rdi_remove_named(name);
            }
        }
    }
}

/// Bridges RDI into a single `IServiceLocator` implementation.
///
/// Resolution order for `get_any`:
/// 1. RDI provider (compile-time services)
///
/// Resolution order for `get_any_named`:
/// 1. Named registry in RDI provider
pub struct ServiceLocatorBridge {
    provider: Arc<RdiProvider>,
}

impl ServiceLocatorBridge {
    pub fn new(provider: Arc<RdiProvider>) -> Self {
        Self { provider }
    }

    pub fn provider(&self) -> &Arc<RdiProvider> {
        &self.provider
    }
}

impl IServiceLocator for ServiceLocatorBridge {
    fn get_any(&self, type_key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.provider.get_any(type_key)
    }

    fn get_any_named(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.provider.get_named_any(name)
    }

    fn register_named_any(&self, name: &str, service: Arc<dyn Any + Send + Sync>) {
        self.provider.register_named_any(name, service);
    }

    fn remove_named(&self, name: &str) {
        self.provider.remove_named(name);
    }
}

impl INamedRegistrar for ServiceLocatorBridge {
    fn register_named_any(&self, name: &str, service: Arc<dyn Any + Send + Sync>) {
        self.provider.register_named_any(name, service);
    }

    fn remove_named(&self, name: &str) {
        self.provider.remove_named(name);
    }
}

/// Implements `IServiceLocator` for standalone RDI types that don't need
/// the named registry.
macro_rules! impl_service_locator {
    ($ty:ty) => {
        impl IServiceLocator for $ty {
            fn get_any(&self, type_key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
                IServiceResolver::get_any(self, type_key)
            }

            fn get_any_named(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
                // Standalone providers may have named services registered.
                self.get_named_any(name)
            }

            fn register_named_any(&self, name: &str, service: Arc<dyn Any + Send + Sync>) {
                self.rdi_register_named(name, service);
            }

            fn remove_named(&self, name: &str) {
                self.rdi_remove_named(name);
            }
        }
    };
}

impl_service_locator!(ServiceProvider);
impl_service_locator!(crate::scope::Scope);
impl_service_locator!(ServiceProviderWrapper);
