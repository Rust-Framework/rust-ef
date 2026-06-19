mod common;

use rust_dicore::*;
use std::sync::Arc;

#[test]
fn create_scope_medinaming() {
    let p = Arc::new(
        ServiceCollection::new()
            .scoped(|_| Arc::new(common::MyService { value: 10 }))
            .build()
            .unwrap(),
    );
    let scope = p.create_scope();
    let svc: Arc<common::MyService> = scope.get();
    assert_eq!(svc.value, 10);
}

#[test]
fn scope_get_service_and_get_required() {
    let p = Arc::new(
        ServiceCollection::new()
            .scoped(|_| Arc::new(common::MyService { value: 20 }))
            .build()
            .unwrap(),
    );
    let scope = p.scope();
    let opt: Option<Arc<common::MyService>> = scope.get_service();
    assert_eq!(opt.unwrap().value, 20);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Arc<common::Logger> = scope.get_required_service();
    }));
    assert!(result.is_err());
}

#[test]
fn scope_get_services_returns_empty() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    let scope = p.scope();
    let all: Vec<Arc<common::MyService>> = scope.get_services();
    assert!(all.is_empty());
}

#[test]
fn scope_get_named_any_delegates_to_parent() {
    let p = Arc::new(ServiceCollection::new().build().unwrap());
    p.register_named("scope_test", Arc::new(common::MyService { value: 5 }));
    let scope = p.scope();
    let retrieved = scope.get_named_any("scope_test");
    assert!(retrieved.is_some());
}

#[test]
fn scoped_cached_within_same_scope() {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    use std::sync::atomic::Ordering;
    let p = Arc::new(
        ServiceCollection::new()
            .scoped(|_| {
                COUNTER.fetch_add(1, Ordering::SeqCst);
                Arc::new(common::MyService { value: 1 })
            })
            .build()
            .unwrap(),
    );
    let scope = p.scope();
    let _a = scope.get::<common::MyService>();
    let _b = scope.get::<common::MyService>();
    assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
}
