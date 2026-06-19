mod common;

use rust_dicore::*;
use std::sync::Arc;

#[test]
fn named_service_roundtrip() {
    let p = ServiceCollection::new().build().unwrap();
    let svc = Arc::new(common::MyService { value: 7 });
    p.register_named("my_svc", svc.clone());
    let retrieved: Arc<common::MyService> = p.get_named("my_svc").unwrap();
    assert_eq!(retrieved.value, 7);
    assert!(Arc::ptr_eq(&svc, &retrieved));
}

#[test]
fn named_service_remove() {
    let p = ServiceCollection::new().build().unwrap();
    p.register_named("tmp", Arc::new(common::MyService { value: 1 }));
    p.remove_named("tmp");
    assert!(p.get_named::<common::MyService>("tmp").is_none());
}

#[test]
fn named_service_not_found_returns_none() {
    let p = ServiceCollection::new().build().unwrap();
    assert!(p.get_named::<common::MyService>("nonexistent").is_none());
}

#[test]
fn get_named_any_returns_none_on_missing() {
    let p = ServiceCollection::new().build().unwrap();
    assert!(p.get_named_any("nothing").is_none());
}

#[test]
fn named_survives_scope_drop() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    p.register_named("persist", Arc::new(common::MyService { value: 999 }));
    {
        let scope = p.scope();
        let retrieved = scope.get_named_any("persist");
        assert!(retrieved.is_some());
    }
    let still_there: Arc<common::MyService> = p.get_named("persist").unwrap();
    assert_eq!(still_there.value, 999);
}
