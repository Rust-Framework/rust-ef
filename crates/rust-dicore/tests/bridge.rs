mod common;

use rust_dicore::*;
use std::any::Any;
use std::sync::Arc;

#[test]
fn rdiprovider_root_resolves() {
    let p = Arc::new(
        ServiceCollection::new()
            .singleton(|_| Arc::new(common::MyService { value: 100 }))
            .build()
            .unwrap(),
    );
    let rp = RdiProvider::Root(p);
    let svc: Arc<common::MyService> = rp.get();
    assert_eq!(svc.value, 100);
}

#[test]
fn rdiprovider_wrapped_resolves() {
    let root = Arc::new(
        ServiceCollection::new()
            .singleton(|_| Arc::new(common::MyService { value: 200 }))
            .build()
            .unwrap(),
    );
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    let wrapper = ServiceProviderWrapper::new(child, root);
    let rp = RdiProvider::Wrapped(wrapper);
    let svc: Arc<common::MyService> = rp.get();
    assert_eq!(svc.value, 200);
}

#[test]
fn rdiprovider_get_named_any() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    p.register_named("rdi_key", Arc::new(common::MyService { value: 5 }));
    let rp = RdiProvider::Root(p);
    let retrieved = rp.get_named_any("rdi_key");
    assert!(retrieved.is_some());
}

#[test]
fn rdiprovider_register_and_remove_named() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let rp = RdiProvider::Root(p);
    let svc = Arc::new(common::MyService { value: 3 });
    rp.register_named_any("test", svc);
    let retrieved = rp.get_named_any("test");
    assert!(retrieved.is_some());
    let down: Arc<common::MyService> = retrieved.unwrap().downcast::<common::MyService>().unwrap();
    assert_eq!(down.value, 3);
    rp.remove_named("test");
    assert!(rp.get_named_any("test").is_none());
}

#[test]
fn servicelocatorbridge_roundtrip() {
    let p = Arc::new(
        ServiceCollection::new()
            .singleton(|_| Arc::new(common::MyService { value: 55 }))
            .build()
            .unwrap(),
    );
    let rp = Arc::new(RdiProvider::Root(p));
    let bridge = ServiceLocatorBridge::new(rp);
    let result = bridge.get_any(std::any::type_name::<common::MyService>());
    assert!(result.is_some());
    let named_svc: Arc<dyn Any + Send + Sync> = Arc::new(Arc::new(common::MyService { value: 77 }));
    IServiceLocator::register_named_any(&bridge, "bridge_key", named_svc);
    let retrieved = bridge.get_any_named("bridge_key");
    assert!(retrieved.is_some());
    IServiceLocator::remove_named(&bridge, "bridge_key");
    assert!(bridge.get_any_named("bridge_key").is_none());
}

#[test]
fn servicelocatorbridge_provider_accessor() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let rp = Arc::new(RdiProvider::Root(p.clone()));
    let bridge = ServiceLocatorBridge::new(rp.clone());
    assert!(Arc::ptr_eq(&rp, bridge.provider()));
}

#[test]
fn serviceprovider_as_servicelocator_get_any() {
    let p = Arc::new(
        ServiceCollection::new()
            .singleton(|_| Arc::new(common::MyService { value: 1 }))
            .build()
            .unwrap(),
    );
    let locator: &dyn IServiceLocator = p.as_ref();
    let result = locator.get_any(std::any::type_name::<common::MyService>());
    assert!(result.is_some());
}

#[test]
fn serviceprovider_as_servicelocator_named() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    p.register_named("loc_svc", Arc::new(common::MyService { value: 2 }));
    let locator: &dyn IServiceLocator = p.as_ref();
    let result = locator.get_any_named("loc_svc");
    assert!(result.is_some());
}

#[test]
fn serviceprovider_as_servicelocator_register_remove() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let locator: &dyn IServiceLocator = p.as_ref();
    let svc: Arc<dyn Any + Send + Sync> = Arc::new(Arc::new(common::MyService { value: 3 }));
    locator.register_named_any("loc_reg", svc);
    assert!(locator.get_any_named("loc_reg").is_some());
    locator.remove_named("loc_reg");
    assert!(locator.get_any_named("loc_reg").is_none());
}

#[test]
fn named_registrar_via_bridge() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let rp = Arc::new(RdiProvider::Root(p));
    let bridge = ServiceLocatorBridge::new(rp);
    let registrar: &dyn INamedRegistrar = &bridge;
    let svc: Arc<dyn Any + Send + Sync> = Arc::new(Arc::new(common::MyService { value: 10 }));
    registrar.register_named_any("reg", svc);
    assert!(bridge.get_any_named("reg").is_some());
    registrar.remove_named("reg");
    assert!(bridge.get_any_named("reg").is_none());
}
