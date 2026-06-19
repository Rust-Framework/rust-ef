use crate::lifetime::ServiceLifetime;
use std::any::Any;
use std::sync::Arc;

/// Service resolver trait �?the core DI resolution interface.
///
/// Provides both type-erased and generic resolution methods.
pub trait IServiceResolver: Send + Sync {
    fn get_any(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>>;
    fn get_keyed_any(&self, key: &str, variant: &str) -> Option<Arc<dyn Any + Send + Sync>>;

    /// Resolve a service by type (concrete or `dyn Trait`).
    /// Panics if not registered.
    fn get<T: ?Sized + Sync + Send + 'static>(&self) -> Arc<T>
    where
        Self: Sized,
    {
        self.get_any(std::any::type_name::<T>())
            .and_then(|a| a.downcast::<Arc<T>>().ok().map(|d| Arc::clone(&*d)))
            .unwrap_or_else(|| panic!("service not registered: {}", std::any::type_name::<T>()))
    }

    /// Resolve a service by type, returning `None` if not registered.
    fn try_get<T: ?Sized + Sync + Send + 'static>(&self) -> Option<Arc<T>>
    where
        Self: Sized,
    {
        self.get_any(std::any::type_name::<T>())
            .and_then(|a| a.downcast::<Arc<T>>().ok().map(|d| Arc::clone(&*d)))
    }

    /// Resolve a keyed service by type and key. Panics if not found.
    fn get_keyed<T: ?Sized + Sync + Send + 'static>(&self, variant: &str) -> Arc<T>
    where
        Self: Sized,
    {
        self.get_keyed_any(std::any::type_name::<T>(), variant)
            .and_then(|a| a.downcast::<Arc<T>>().ok().map(|d| Arc::clone(&*d)))
            .unwrap_or_else(|| {
                panic!(
                    "keyed service not registered: {}:{}",
                    std::any::type_name::<T>(),
                    variant
                )
            })
    }

    /// Resolve a keyed service by type and key, returning `None` if not found.
    fn try_get_keyed<T: ?Sized + Sync + Send + 'static>(&self, variant: &str) -> Option<Arc<T>>
    where
        Self: Sized,
    {
        self.get_keyed_any(std::any::type_name::<T>(), variant)
            .and_then(|a| a.downcast::<Arc<T>>().ok().map(|d| Arc::clone(&*d)))
    }
}

pub struct ServiceEntry {
    pub cache_key: usize,
    pub key: Option<String>,
    pub type_name: &'static str, // kept for IServiceLocator string-based resolution
    pub factory: ServiceFactory,
    pub lifetime: ServiceLifetime,
}

pub type ServiceFactory =
    Arc<dyn Fn(&dyn IServiceResolver) -> Arc<dyn Any + Send + Sync> + Send + Sync>;
