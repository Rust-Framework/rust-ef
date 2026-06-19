use crate::entry::{IServiceResolver, ServiceEntry, ServiceFactory};
use crate::error::RdiError;
use crate::lifetime::ServiceLifetime;
use crate::store::ServiceStore;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct ServiceProvider {
    store: ServiceStore,
    /// String �?TypeId lookup for IServiceLocator string-based resolution.
    type_map: HashMap<&'static str, TypeId>,
    /// Eager-executed singleton cache. Indexed by cache_key.
    /// Non-singleton entries are not present.
    /// Uses RwLock for interior mutability: during eager initialization,
    /// a singleton factory may reference another singleton not yet populated.
    /// The lazy fallback in get_any_by_entry handles this by executing the
    /// factory on-demand and caching the result.
    singleton_cache: RwLock<HashMap<usize, Arc<dyn Any + Send + Sync>>>,
    /// String-keyed registry for cross-DLL (cdylib) service access.
    /// Rust's `TypeId` differs across compilation units, so named
    /// lookup is the only reliable mechanism for plugin services.
    pub(crate) named: RwLock<HashMap<String, Arc<dyn Any + Send + Sync>>>,
}

impl ServiceProvider {
    pub(crate) fn new(store: ServiceStore) -> Result<Self, RdiError> {
        // Build type_name �?TypeId lookup table for string-based resolution
        let mut type_map = HashMap::new();
        for (&tid, entries) in &store {
            if let Some(e) = entries.first() {
                type_map.entry(e.type_name).or_insert(tid);
            }
        }

        // Two-phase singleton initialization to support cross-references.
        // Phase 1: collect all singleton entries.
        let singleton_entries: Vec<(usize, ServiceFactory)> = store
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|e| e.lifetime == ServiceLifetime::Singleton)
            .map(|e| (e.cache_key, e.factory.clone()))
            .collect();

        // Phase 2: eagerly execute all singleton factories.
        // If a singleton factory references another singleton not yet
        // populated, the lazy fallback in get_any_by_entry handles it
        // by executing the factory on-demand via interior mutability.
        let sp = Self {
            store,
            type_map,
            singleton_cache: RwLock::new(HashMap::new()),
            named: RwLock::new(HashMap::new()),
        };

        for (ck, factory) in &singleton_entries {
            let instance = (factory)(&sp as &dyn IServiceResolver);
            sp.singleton_cache.write().unwrap().insert(*ck, instance);
        }

        Ok(sp)
    }

    /// Resolve a service by type. Works uniformly for concrete types and trait objects.
    /// Panics if not registered.
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Arc<T> {
        self.try_get::<T>()
            .unwrap_or_else(|| panic!("service not registered: {}", std::any::type_name::<T>()))
    }

    /// Resolve a service by type, returning `None` if not registered.
    pub fn get_optional<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.try_get::<T>()
    }

    /// Resolve a keyed service by type and key. Panics if not found.
    pub fn get_keyed<T: ?Sized + Send + Sync + 'static>(&self, key: &str) -> Arc<T> {
        self.try_get_keyed::<T>(key).unwrap_or_else(|| {
            panic!(
                "keyed service not registered: {}:{}",
                std::any::type_name::<T>(),
                key
            )
        })
    }

    /// Return all registered instances of the given type.
    pub fn get_all<T: ?Sized + Send + Sync + 'static>(&self) -> Vec<Arc<T>> {
        let tid = TypeId::of::<T>();
        match self.store.get(&tid) {
            Some(entries) => entries
                .iter()
                .filter_map(|e| {
                    let arc = self.get_any_by_entry(e)?;
                    Self::extract(arc)
                })
                .collect(),
            None => Vec::new(),
        }
    }

    /// Create a new service scope.
    ///
    /// Analogous to `IServiceProvider.CreateScope()` in MEDI.
    /// Scoped-lifetime services are cached within the returned scope.
    pub fn scope(self: &Arc<Self>) -> crate::scope::Scope {
        crate::scope::Scope::new(self.clone())
    }

    /// Alias for `scope()` with MEDI-inspired naming.
    pub fn create_scope(self: &Arc<Self>) -> crate::scope::Scope {
        self.scope()
    }

    /// Alias for `get()` with MEDI-inspired naming (`GetService<T>()`).
    pub fn get_service<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.get_optional::<T>()
    }

    /// Resolve a service by type, panicking if not registered.
    /// MEDI-inspired naming (`GetRequiredService<T>()`).
    pub fn get_required_service<T: ?Sized + Send + Sync + 'static>(&self) -> Arc<T> {
        self.get::<T>()
    }

    /// Return all registered instances of the given type.
    /// MEDI-inspired naming (`GetServices<T>()`).
    pub fn get_services<T: ?Sized + Send + Sync + 'static>(&self) -> Vec<Arc<T>> {
        self.get_all::<T>()
    }

    fn try_get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let tid = TypeId::of::<T>();
        let entry = self.store.get(&tid)?.iter().find(|e| e.key.is_none())?;
        let arc = self.get_any_by_entry(entry)?;
        Self::extract(arc)
    }

    fn try_get_keyed<T: ?Sized + Send + Sync + 'static>(&self, key: &str) -> Option<Arc<T>> {
        let tid = TypeId::of::<T>();
        let entry = self
            .store
            .get(&tid)?
            .iter()
            .find(|e| e.key.as_deref() == Some(key))?;
        let arc = self.get_any_by_entry(entry)?;
        Self::extract(arc)
    }

    pub(crate) fn get_any_by_entry(
        &self,
        entry: &ServiceEntry,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        match entry.lifetime {
            ServiceLifetime::Singleton => {
                // Check eager cache (populated at build time).
                // If not found (e.g. cross-reference during eager init), execute
                // the factory on-demand as a lazy fallback.
                {
                    let cache = self.singleton_cache.read().unwrap();
                    if let Some(instance) = cache.get(&entry.cache_key) {
                        return Some(instance.clone());
                    }
                }
                // Lazy fallback: execute factory on-demand and cache.
                let instance = (entry.factory)(self);
                self.singleton_cache
                    .write()
                    .unwrap()
                    .insert(entry.cache_key, instance.clone());
                Some(instance)
            }
            ServiceLifetime::Transient | ServiceLifetime::Scoped => Some((entry.factory)(self)),
        }
    }

    /// Extract `Arc<T>` from `Arc<Arc<T>>` stored inside `Arc<dyn Any>`.
    /// The factory double-wraps: inner `Arc<T>`, outer `Arc<dyn Any>`.
    pub(crate) fn extract<T: ?Sized + Send + Sync + 'static>(
        arc: Arc<dyn Any + Send + Sync>,
    ) -> Option<Arc<T>> {
        let double: Arc<Arc<T>> = arc.downcast::<Arc<T>>().ok()?;
        Some(Arc::clone(&*double))
    }

    /// Get entries by TypeId (used internally and by Scope).
    pub(crate) fn entries_by_tid(&self, tid: &TypeId) -> Option<&Vec<ServiceEntry>> {
        self.store.get(tid)
    }

    /// Find entry by string type_name + variant (for string-based resolution).
    pub(crate) fn entry_by_str(&self, type_name: &str, variant: &str) -> Option<&ServiceEntry> {
        let tid = self.type_map.get(type_name)?;
        self.store
            .get(tid)?
            .iter()
            .find(|e| e.key.as_deref() == Some(variant))
    }

    /// Get entries by string type_name (for string-based resolution).
    pub(crate) fn entries_by_str(&self, type_name: &str) -> Option<&Vec<ServiceEntry>> {
        let tid = self.type_map.get(type_name)?;
        self.store.get(tid)
    }

    /// Cross-DLL safe named service resolution (generic).
    pub fn get_named<T: Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        self.named
            .read()
            .unwrap()
            .get(name)?
            .clone()
            .downcast::<T>()
            .ok()
    }

    /// Non-generic named resolution; returns `Arc<dyn Any>` for trait-object dispatch.
    pub fn get_named_any(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.named.read().unwrap().get(name).cloned()
    }

    /// Register a named service for cross-DLL plugin access.
    pub fn register_named<T: Send + Sync + 'static>(&self, name: &str, service: Arc<T>) {
        self.named
            .write()
            .unwrap()
            .insert(name.to_string(), service);
    }

    /// Remove a named service (for plugin unload).
    pub fn remove_named(&self, name: &str) {
        self.named.write().unwrap().remove(name);
    }

    /// Register a named service (for `impl_service_locator!` macro).
    pub fn rdi_register_named(&self, name: &str, service: Arc<dyn Any + Send + Sync>) {
        self.named
            .write()
            .unwrap()
            .insert(name.to_string(), service);
    }

    /// Remove a named service (for `impl_service_locator!` macro).
    pub fn rdi_remove_named(&self, name: &str) {
        self.named.write().unwrap().remove(name);
    }
}

impl IServiceResolver for ServiceProvider {
    fn get_any(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        let tid = self.type_map.get(key)?;
        let entry = self.store.get(tid)?.iter().find(|e| e.key.is_none())?;
        self.get_any_by_entry(entry)
    }
    fn get_keyed_any(&self, key: &str, variant: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        let entry = self.entry_by_str(key, variant)?;
        self.get_any_by_entry(entry)
    }
}

#[cfg(test)]
mod tests {
    use crate::collection::ServiceCollection;
    use std::sync::Arc;

    #[derive(Debug, PartialEq)]
    struct Calc(i32);
    #[test]
    fn optional_missing() {
        let p = ServiceCollection::new().build().unwrap();
        assert!(p.get_optional::<i32>().is_none());
    }
    #[test]
    fn all_basic() {
        let p = ServiceCollection::new()
            .keyed("a", |_| Arc::new(Calc(1)))
            .keyed("b", |_| Arc::new(Calc(2)))
            .build()
            .unwrap();
        assert_eq!(p.get_all::<Calc>().len(), 2);
    }
}
