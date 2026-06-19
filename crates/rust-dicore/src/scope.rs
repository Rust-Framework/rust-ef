use crate::entry::IServiceResolver;
use crate::lifetime::ServiceLifetime;
use crate::provider::ServiceProvider;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct Scope {
    parent: Arc<ServiceProvider>,
    scoped_cache: RwLock<HashMap<usize, Arc<dyn Any + Send + Sync>>>,
}

impl Scope {
    pub(crate) fn new(parent: Arc<ServiceProvider>) -> Self {
        Self {
            parent,
            scoped_cache: RwLock::new(HashMap::new()),
        }
    }

    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Arc<T> {
        self.try_get::<T>()
            .unwrap_or_else(|| panic!("service not registered: {}", std::any::type_name::<T>()))
    }

    pub fn get_optional<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.try_get::<T>()
    }

    /// Alias for `get_optional()` with MEDI-inspired naming.
    pub fn get_service<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.get_optional::<T>()
    }

    /// Alias for `get()` with MEDI-inspired naming.
    pub fn get_required_service<T: ?Sized + Send + Sync + 'static>(&self) -> Arc<T> {
        self.get::<T>()
    }

    /// Return all registered instances of the given type.
    /// MEDI-inspired naming.
    pub fn get_services<T: ?Sized + Send + Sync + 'static>(&self) -> Vec<Arc<T>> {
        self.get_all::<T>()
    }

    pub fn get_keyed<T: ?Sized + Send + Sync + 'static>(&self, key: &str) -> Arc<T> {
        self.try_get_keyed::<T>(key).unwrap_or_else(|| {
            panic!(
                "keyed service not registered: {}:{}",
                std::any::type_name::<T>(),
                key
            )
        })
    }

    pub fn get_all<T: ?Sized + Send + Sync + 'static>(&self) -> Vec<Arc<T>> {
        let tid = TypeId::of::<T>();
        if let Some(entries) = self.parent.entries_by_tid(&tid) {
            entries
                .iter()
                .filter_map(|e| {
                    let arc = self.get_any_by_entry(e)?;
                    ServiceProvider::extract(arc)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub fn get_named_any(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.parent.get_named_any(name)
    }

    fn try_get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let tid = TypeId::of::<T>();
        let entry = self
            .parent
            .entries_by_tid(&tid)?
            .iter()
            .find(|e| e.key.is_none())?;
        let arc = self.get_any_by_entry(entry)?;
        ServiceProvider::extract(arc)
    }

    fn try_get_keyed<T: ?Sized + Send + Sync + 'static>(&self, key: &str) -> Option<Arc<T>> {
        let tid = TypeId::of::<T>();
        let entries = self.parent.entries_by_tid(&tid)?;
        let entry = entries.iter().find(|e| e.key.as_deref() == Some(key))?;
        let arc = self.get_any_by_entry(entry)?;
        ServiceProvider::extract(arc)
    }

    fn get_any_by_entry(
        &self,
        entry: &crate::entry::ServiceEntry,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        match entry.lifetime {
            ServiceLifetime::Singleton => {
                // Singleton cache lives in parent's eager cache
                self.parent.get_any_by_entry(entry)
            }
            ServiceLifetime::Transient => Some((entry.factory)(self.parent.as_ref())),
            ServiceLifetime::Scoped => {
                {
                    let cache = self.scoped_cache.read().unwrap();
                    if let Some(instance) = cache.get(&entry.cache_key) {
                        return Some(instance.clone());
                    }
                }
                let instance = (entry.factory)(self);
                {
                    self.scoped_cache
                        .write()
                        .unwrap()
                        .insert(entry.cache_key, instance.clone());
                }
                Some(instance)
            }
        }
    }
}

impl IServiceResolver for Scope {
    fn get_any(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        if let Some(entries) = self.parent.entries_by_str(key) {
            for entry in entries {
                if entry.key.is_none() {
                    if let Some(r) = self.get_any_by_entry(entry) {
                        return Some(r);
                    }
                }
            }
        }
        None
    }
    fn get_keyed_any(&self, key: &str, variant: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        let entry = self.parent.entry_by_str(key, variant)?;
        self.get_any_by_entry(entry)
    }
}

impl Scope {
    /// Register a named service (for `impl_service_locator!` macro).
    pub fn rdi_register_named(&self, name: &str, service: Arc<dyn Any + Send + Sync>) {
        self.parent.rdi_register_named(name, service);
    }

    /// Remove a named service (for `impl_service_locator!` macro).
    pub fn rdi_remove_named(&self, name: &str) {
        self.parent.rdi_remove_named(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::ServiceCollection;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[derive(Debug, PartialEq)]
    struct Sd(u64);
    #[test]
    fn scoped_cached_per_scope() {
        static NXT: AtomicU64 = AtomicU64::new(0);
        let p = Arc::new(
            ServiceCollection::new()
                .scoped(|_| Arc::new(Sd(NXT.fetch_add(1, Ordering::SeqCst))))
                .build()
                .unwrap(),
        );
        let s1 = p.scope();
        let a = s1.get::<Sd>();
        let b = s1.get::<Sd>();
        assert_eq!(a.0, b.0);
        let s2 = p.scope();
        let c = s2.get::<Sd>();
        assert_ne!(a.0, c.0);
    }
}
