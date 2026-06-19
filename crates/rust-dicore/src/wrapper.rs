use crate::entry::IServiceResolver;
use crate::provider::ServiceProvider;
use std::any::Any;
use std::sync::Arc;

pub struct ServiceProviderWrapper {
    child: Arc<ServiceProvider>,
    root: Arc<ServiceProvider>,
}

impl ServiceProviderWrapper {
    pub fn new(child: Arc<ServiceProvider>, root: Arc<ServiceProvider>) -> Arc<Self> {
        Arc::new(Self { child, root })
    }
    pub fn child(&self) -> &Arc<ServiceProvider> {
        &self.child
    }
    pub fn root(&self) -> &Arc<ServiceProvider> {
        &self.root
    }

    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Arc<T> {
        self.try_get::<T>()
            .unwrap_or_else(|| panic!("service not registered: {}", std::any::type_name::<T>()))
    }
    pub fn get_optional<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.try_get::<T>()
    }
    pub fn get_keyed<T: ?Sized + Send + Sync + 'static>(&self, key: &str) -> Arc<T> {
        self.try_get_keyed::<T>(key).unwrap_or_else(|| {
            panic!(
                "keyed service not found: {}:{}",
                std::any::type_name::<T>(),
                key
            )
        })
    }
    pub fn get_all<T: ?Sized + Send + Sync + 'static>(&self) -> Vec<Arc<T>> {
        let mut r = self.child.get_all::<T>();
        r.extend(self.root.get_all::<T>());
        r
    }
    pub fn get_named<T: Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        self.child
            .get_named::<T>(name)
            .or_else(|| self.root.get_named::<T>(name))
    }
    pub fn get_named_any(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        self.child
            .get_named_any(name)
            .or_else(|| self.root.get_named_any(name))
    }

    fn try_get<T: ?Sized + Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.child
            .get_optional::<T>()
            .or_else(|| self.root.get_optional::<T>())
    }

    fn try_get_keyed<T: ?Sized + Send + Sync + 'static>(&self, key: &str) -> Option<Arc<T>> {
        // Delegate to IServiceResolver which goes through the cache properly.
        IServiceResolver::get_keyed_any(self.child.as_ref(), std::any::type_name::<T>(), key)
            .or_else(|| {
                IServiceResolver::get_keyed_any(self.root.as_ref(), std::any::type_name::<T>(), key)
            })
            .and_then(|arc| crate::provider::ServiceProvider::extract(arc))
    }
}

impl IServiceResolver for ServiceProviderWrapper {
    fn get_any(&self, key: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        IServiceResolver::get_any(self.child.as_ref(), key)
            .or_else(|| IServiceResolver::get_any(self.root.as_ref(), key))
    }
    fn get_keyed_any(&self, key: &str, variant: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        IServiceResolver::get_keyed_any(self.child.as_ref(), key, variant)
            .or_else(|| IServiceResolver::get_keyed_any(self.root.as_ref(), key, variant))
    }
}

impl ServiceProviderWrapper {
    /// Register a named service (for `impl_service_locator!` macro).
    pub fn rdi_register_named(&self, name: &str, service: Arc<dyn Any + Send + Sync>) {
        self.child.rdi_register_named(name, service);
    }

    /// Remove a named service (for `impl_service_locator!` macro).
    pub fn rdi_remove_named(&self, name: &str) {
        self.child.rdi_remove_named(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::ServiceCollection;
    #[derive(Debug, PartialEq)]
    struct PO;
    #[derive(Debug, PartialEq)]
    struct RO {
        v: i32,
    }
    #[derive(Debug, PartialEq)]
    struct B {
        s: String,
    }
    #[test]
    fn child_prio() {
        let r = Arc::new(
            ServiceCollection::new()
                .singleton(|_| Arc::new(B { s: "root".into() }))
                .build()
                .unwrap(),
        );
        let c = ServiceCollection::new()
            .singleton(|_| Arc::new(B { s: "child".into() }))
            .build()
            .unwrap();
        let w = ServiceProviderWrapper::new(Arc::new(c), r);
        assert_eq!(w.get::<B>().s, "child");
    }
    #[test]
    fn root_fallback() {
        let r = Arc::new(
            ServiceCollection::new()
                .singleton(|_| Arc::new(RO { v: 42 }))
                .build()
                .unwrap(),
        );
        let c = ServiceCollection::new().build().unwrap();
        let w = ServiceProviderWrapper::new(Arc::new(c), r);
        assert_eq!(w.get::<RO>().v, 42);
    }
    #[test]
    fn child_invisible() {
        let r = Arc::new(ServiceCollection::new().build().unwrap());
        let c = ServiceCollection::new()
            .singleton(|_| Arc::new(PO))
            .build()
            .unwrap();
        let _w = ServiceProviderWrapper::new(Arc::new(c), r.clone());
        assert!(r.get_optional::<PO>().is_none());
    }
}
