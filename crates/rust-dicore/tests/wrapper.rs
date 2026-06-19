mod common;

use rust_dicore::*;
use std::sync::Arc;

#[test]
fn wrapper_get_keyed_fallback_to_root() {
    let root = Arc::new(
        ServiceCollection::new()
            .keyed("a", |_| {
                Arc::new(common::Logger {
                    prefix: "root".into(),
                })
            })
            .build()
            .unwrap(),
    );
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    let w = ServiceProviderWrapper::new(child, root);
    let svc: Arc<common::Logger> = w.get_keyed("a");
    assert_eq!(svc.prefix, "root");
}

#[test]
fn wrapper_get_named_child_first() {
    let root = Arc::new(ServiceCollection::new().build().unwrap());
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    let root_svc = Arc::new(common::Logger {
        prefix: "root".into(),
    });
    let child_svc = Arc::new(common::Logger {
        prefix: "child".into(),
    });
    root.register_named("log", root_svc);
    child.register_named("log", child_svc.clone());
    let w = ServiceProviderWrapper::new(child.clone(), root.clone());
    let retrieved: Arc<common::Logger> = w.get_named("log").unwrap();
    assert_eq!(retrieved.prefix, "child");
}

#[test]
fn wrapper_get_named_fallback_to_root() {
    let root = Arc::new(ServiceCollection::new().build().unwrap());
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    root.register_named("shared", Arc::new(common::MyService { value: 99 }));
    let w = ServiceProviderWrapper::new(child, root);
    let retrieved: Arc<common::MyService> = w.get_named("shared").unwrap();
    assert_eq!(retrieved.value, 99);
}

#[test]
fn wrapper_get_named_any_child_first() {
    let root = Arc::new(ServiceCollection::new().build().unwrap());
    let child = Arc::new(ServiceCollection::new().build().unwrap());
    root.register_named("key", Arc::new(common::MyService { value: 1 }));
    child.register_named("key", Arc::new(common::MyService { value: 2 }));
    let w = ServiceProviderWrapper::new(child, root);
    let retrieved = w.get_named_any("key").unwrap();
    let svc: Arc<common::MyService> = retrieved.downcast::<common::MyService>().unwrap();
    assert_eq!(svc.value, 2);
}

#[test]
fn wrapper_get_all_combines_both() {
    let root = Arc::new(
        ServiceCollection::new()
            .keyed("a", |_| Arc::new(common::MyService { value: 1 }))
            .build()
            .unwrap(),
    );
    let child = Arc::new(
        ServiceCollection::new()
            .keyed("b", |_| Arc::new(common::MyService { value: 2 }))
            .build()
            .unwrap(),
    );
    let w = ServiceProviderWrapper::new(child, root);
    let all: Vec<Arc<common::MyService>> = w.get_all();
    assert_eq!(all.len(), 2);
}
