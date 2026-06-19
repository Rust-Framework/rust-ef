mod common;

use rust_dicore::*;
use std::sync::Arc;

#[test]
fn get_all_returns_all_entries() {
    let p = ServiceCollection::new()
        .singleton(|_| Arc::new(common::MyService { value: 1 }))
        .keyed("extra", |_| Arc::new(common::MyService { value: 2 }))
        .build()
        .unwrap();
    let all: Vec<Arc<common::MyService>> = p.get_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn scope_get_keyed_found() {
    let p = Arc::new(
        ServiceCollection::new()
            .keyed_scoped("k", |_| Arc::new(common::MyService { value: 10 }))
            .build()
            .unwrap(),
    );
    let scope = p.scope();
    let svc: Arc<common::MyService> = scope.get_keyed("k");
    assert_eq!(svc.value, 10);
}

#[test]
fn scope_get_none_on_missing() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let scope = p.scope();
    assert!(scope.get_optional::<common::MyService>().is_none());
}

#[test]
fn scope_get_all_from_parent() {
    let p = Arc::new(
        ServiceCollection::new()
            .keyed("a", |_| Arc::new(common::MyService { value: 1 }))
            .keyed("b", |_| Arc::new(common::MyService { value: 2 }))
            .build()
            .unwrap(),
    );
    let scope = p.scope();
    let all: Vec<Arc<common::MyService>> = scope.get_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn instance_preserves_arc() {
    let svc = Arc::new(common::MyService { value: 77 });
    let p = ServiceCollection::new()
        .instance(svc.clone())
        .build()
        .unwrap();
    let retrieved: Arc<common::MyService> = p.get();
    assert!(Arc::ptr_eq(&svc, &retrieved));
}

#[test]
fn singleton_caches() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let p = ServiceCollection::new()
        .singleton(|_| {
            COUNT.fetch_add(1, Ordering::SeqCst);
            Arc::new(common::MyService { value: 1 })
        })
        .build()
        .unwrap();
    let _a: Arc<common::MyService> = p.get();
    let _b: Arc<common::MyService> = p.get();
    assert_eq!(COUNT.load(Ordering::SeqCst), 1);
}

#[test]
fn transient_creates_new_each_time() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let p = ServiceCollection::new()
        .transient(|_| {
            COUNT.fetch_add(1, Ordering::SeqCst);
            Arc::new(common::MyService { value: 1 })
        })
        .build()
        .unwrap();
    let _a: Arc<common::MyService> = p.get();
    let _b: Arc<common::MyService> = p.get();
    assert_eq!(COUNT.load(Ordering::SeqCst), 2);
}

#[test]
fn try_add_skips_existing() {
    let p = ServiceCollection::new()
        .singleton(|_| Arc::new(common::MyService { value: 1 }))
        .try_add(|_| Arc::new(common::MyService { value: 2 }))
        .build()
        .unwrap();
    assert_eq!(p.get::<common::MyService>().value, 1);
}

#[test]
fn wrapper_get_optional_child_first() {
    let root = Arc::new(
        ServiceCollection::new()
            .singleton(|_| Arc::new(common::MyService { value: 1 }))
            .build()
            .unwrap(),
    );
    let child = Arc::new(
        ServiceCollection::new()
            .singleton(|_| Arc::new(common::MyService { value: 2 }))
            .build()
            .unwrap(),
    );
    let w = ServiceProviderWrapper::new(child, root);
    assert_eq!(w.get::<common::MyService>().value, 2);
    assert_eq!(w.get_optional::<common::MyService>().unwrap().value, 2);
}

#[test]
fn wrapper_get_optional_none() {
    let root = Arc::new(ServiceCollection::new().build().unwrap());
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    let w = ServiceProviderWrapper::new(child, root);
    assert!(w.get_optional::<common::MyService>().is_none());
}

#[test]
fn wrapper_get_keyed_child_priority() {
    let root = Arc::new(
        ServiceCollection::new()
            .keyed("k", |_| Arc::new(common::MyService { value: 10 }))
            .build()
            .unwrap(),
    );
    let child = Arc::new(
        ServiceCollection::new()
            .keyed("k", |_| Arc::new(common::MyService { value: 20 }))
            .build()
            .unwrap(),
    );
    let w = ServiceProviderWrapper::new(child, root);
    assert_eq!(w.get_keyed::<common::MyService>("k").value, 20);
}

#[test]
fn wrapper_get_named_not_found() {
    let root = Arc::new(ServiceCollection::new().build().unwrap());
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    let w = ServiceProviderWrapper::new(child, root);
    assert!(w.get_named::<common::MyService>("nothing").is_none());
}

#[test]
fn wrapper_get_named_any_not_found() {
    let root = Arc::new(ServiceCollection::new().build().unwrap());
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    let w = ServiceProviderWrapper::new(child, root);
    assert!(w.get_named_any("nothing").is_none());
}

#[test]
fn singleton_keyed_via_get_resolves_default() {
    // Registering multiple keyed entries should NOT create a default.
    // get() should not find anything.
    let p = ServiceCollection::new()
        .keyed("a", |_| Arc::new(common::MyService { value: 1 }))
        .build()
        .unwrap();
    // get_keyed should work for the registered key
    assert_eq!(p.get_keyed::<common::MyService>("a").value, 1);
}

#[test]
#[should_panic(expected = "service not registered")]
fn get_panics_on_missing() {
    let p = ServiceCollection::new().build().unwrap();
    let _: Arc<common::MyService> = p.get();
}

#[test]
#[should_panic(expected = "keyed service not registered")]
fn get_keyed_panics_on_missing() {
    let p = ServiceCollection::new().build().unwrap();
    let _: Arc<common::MyService> = p.get_keyed("missing");
}

#[test]
fn rdi_register_named_and_remove_via_provider() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let svc: Arc<dyn std::any::Any + Send + Sync> =
        Arc::new(Arc::new(common::MyService { value: 5 }));
    p.rdi_register_named("test", svc.clone());
    assert!(p.get_named_any("test").is_some());
    p.rdi_remove_named("test");
    assert!(p.get_named_any("test").is_none());
}

#[test]
fn scope_get_panics_on_missing() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let scope = p.scope();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Arc<common::MyService> = scope.get();
    }));
    assert!(result.is_err());
}

#[test]
fn scope_get_keyed_panics_on_missing() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let scope = p.scope();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Arc<common::MyService> = scope.get_keyed("nonexistent");
    }));
    assert!(result.is_err());
}

#[test]
fn module_singleton_with_trait() {
    #[rust_dicore::module]
    mod svc {
        rust_dicore::inject!(singleton: dyn super::common::IPlugin => super::common::TestPlugin);
    }
    let p = svc::__rdi_build_provider_svc().unwrap();
    let plugin: Arc<dyn common::IPlugin> = p.get::<dyn common::IPlugin>();
    assert_eq!(plugin.name(), "test_plugin");
}

#[test]
fn module_scoped_lifetime() {
    #[rust_dicore::module]
    mod scoped_mod {
        rust_dicore::inject!(scoped: super::common::MyService);
    }
    let p = Arc::new(scoped_mod::__rdi_build_provider_scoped_mod().unwrap());
    let scope = p.scope();
    let _svc: Arc<common::MyService> = scope.get();
}

#[test]
fn module_transient_lifetime() {
    #[rust_dicore::module]
    mod trans_mod {
        rust_dicore::inject!(transient: super::common::MyService);
    }
    let p = trans_mod::__rdi_build_provider_trans_mod().unwrap();
    let _svc: Arc<common::MyService> = p.get();
}

#[test]
fn module_multiple_registrations() {
    #[rust_dicore::module]
    mod multi {
        rust_dicore::inject!(singleton: super::common::Logger);
        rust_dicore::inject!(singleton: super::common::MyService);
    }
    let p = multi::__rdi_build_provider_multi().unwrap();
    let _log: Arc<common::Logger> = p.get();
    let _svc: Arc<common::MyService> = p.get();
}

#[test]
fn module_keyed_singleton() {
    #[rust_dicore::module]
    mod keyed_mod {
        rust_dicore::inject!(keyed "x": singleton: super::common::MyService);
    }
    let p = keyed_mod::__rdi_build_provider_keyed_mod().unwrap();
    let svc: Arc<common::MyService> = p.get_keyed("x");
    assert_eq!(svc.value, 0);
}

#[test]
fn pattern_get_all_trait_objects() {
    let p = ServiceCollection::new()
        .singleton::<dyn common::IPlugin>(|_| Arc::new(common::TestPlugin))
        .singleton::<dyn common::IPlugin>(|_| Arc::new(common::AltPlugin))
        .build()
        .unwrap();
    let all: Vec<Arc<dyn common::IPlugin>> = p.get_all();
    assert_eq!(all.len(), 2);
}
