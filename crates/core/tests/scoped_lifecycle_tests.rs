//! Scoped lifecycle tests for `add_dbcontext` under rust-dix 0.10.
//!
//! Verifies EFCore-aligned unit-of-work semantics:
//!   - same DI scope        => same DbContext instance (shared resolution)
//!   - different scope      => different instance
//!   - strict scope validation (default): root `get()` of Scoped is rejected
//!   - `ServiceProviderOptions::lenient()` restores 0.7 root-scope caching
//!   - owned resolution always returns a fresh instance (allowed from the root)
//!   - scope/provider disposal rejects further resolution
//!   - scope disposal releases the Scoped instances created by rust-dix

use rust_dix::{RdiError, ServiceKey};
use rust_ef::db_context::DbContext;
use rust_ef::di::{
    DbContextServiceCollectionExt, ServiceCollection, ServiceProvider, ServiceProviderOptions,
};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::Arc;

fn build_provider() -> Arc<ServiceProvider> {
    ServiceCollection::new()
        .add_dbcontext(|o| {
            o.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider")
}

fn build_lenient_provider() -> Arc<ServiceProvider> {
    ServiceCollection::new()
        .add_dbcontext(|o| {
            o.use_sqlite_in_memory();
        })
        .build_with(ServiceProviderOptions::lenient())
        .expect("build lenient provider")
}

#[test]
fn same_scope_returns_same_instance() {
    let provider = build_provider();
    let scope = provider.scope();
    let ctx1: Arc<DbContext> = scope.get().expect("get ctx1");
    let ctx2: Arc<DbContext> = scope.get().expect("get ctx2");
    assert!(
        Arc::ptr_eq(&ctx1, &ctx2),
        "same scope must return same instance (unit-of-work)"
    );
}

#[test]
fn different_scopes_return_different_instances() {
    let provider = build_provider();
    let scope1 = provider.scope();
    let scope2 = provider.scope();
    let ctx1: Arc<DbContext> = scope1.get().expect("get ctx1");
    let ctx2: Arc<DbContext> = scope2.get().expect("get ctx2");
    assert!(
        !Arc::ptr_eq(&ctx1, &ctx2),
        "different scopes must return different instances"
    );
}

#[test]
fn root_provider_rejects_scoped_resolution_by_default() {
    // rust-dix 0.8+ MEDI parity: the root provider is NOT a scope. Resolving a
    // Scoped service from the root must fail instead of silently caching it for
    // the container lifetime.
    let provider = build_provider();

    match provider.get::<DbContext>() {
        Err(RdiError::ScopedFromRootProvider { service }) => {
            assert!(
                service.contains("DbContext"),
                "error must name the scoped service, got {service}"
            );
        }
        Err(other) => panic!("expected ScopedFromRootProvider, got {other:?}"),
        Ok(_) => panic!("root provider must not resolve a Scoped DbContext by default"),
    }
    assert!(
        provider.get_optional::<DbContext>().is_none(),
        "get_optional must treat the scoped service as unresolvable at the root"
    );
}

#[test]
fn lenient_build_restores_root_scope_caching() {
    // Migration escape hatch: `build_with(ServiceProviderOptions::lenient())`
    // restores rust-dix 0.7 behavior (root caches scoped services).
    let provider = build_lenient_provider();
    let ctx1: Arc<DbContext> = provider.get().expect("get ctx1");
    let ctx2: Arc<DbContext> = provider.get().expect("get ctx2");
    assert!(
        Arc::ptr_eq(&ctx1, &ctx2),
        "lenient root resolution must return the same instance (0.7 root scope cache)"
    );
}

#[test]
fn owned_resolution_returns_fresh_instance() {
    // get_owned() bypasses the cache — each call returns a fresh instance.
    // It is allowed from the root provider: the instance is caller-owned and
    // never cached or tracked by the container.
    let provider = build_provider();
    let ctx1: DbContext = provider.get_owned().expect("get_owned ctx1");
    let ctx2: DbContext = provider.get_owned().expect("get_owned ctx2");
    // Owned instances are not Arc, so we compare by address of inner data.
    let addr1 = &ctx1 as *const _ as usize;
    let addr2 = &ctx2 as *const _ as usize;
    assert_ne!(
        addr1, addr2,
        "owned resolution must return a fresh instance each call"
    );
}

#[test]
fn owned_resolution_bypasses_scope_cache() {
    // Even within a scope, get_owned() returns a fresh instance (not cached).
    let provider = build_provider();
    let scope = provider.scope();
    let shared: Arc<DbContext> = scope.get().expect("get shared");
    let owned: DbContext = scope.get_owned().expect("get_owned");
    let shared_addr = Arc::as_ptr(&shared) as usize;
    let owned_addr = &owned as *const _ as usize;
    assert_ne!(
        shared_addr, owned_addr,
        "owned resolution must bypass scope cache"
    );
}

#[test]
fn scope_dispose_rejects_further_resolution() {
    let provider = build_provider();
    let scope = provider.scope();
    let _ctx: Arc<DbContext> = scope.get().expect("get ctx");
    scope.dispose();
    assert!(
        matches!(scope.get::<DbContext>(), Err(RdiError::ScopeDisposed)),
        "resolution after scope.dispose() must fail with ScopeDisposed"
    );
    assert!(scope.get_optional::<DbContext>().is_none());
}

#[test]
fn provider_dispose_rejects_resolution() {
    let provider = build_provider();
    // A live scoped instance does not keep the provider alive/resolvable.
    let _ctx: Arc<DbContext> = provider.scope().get().expect("get ctx");
    provider.dispose();
    assert!(
        matches!(provider.get::<DbContext>(), Err(RdiError::ProviderDisposed)),
        "resolution after provider.dispose() must fail with ProviderDisposed"
    );
    assert!(
        matches!(
            provider.get_owned::<DbContext>(),
            Err(RdiError::ProviderDisposed)
        ),
        "owned resolution after provider.dispose() must fail with ProviderDisposed"
    );
}

#[test]
fn keyed_scoped_context_requires_a_scope_for_shared_resolution() {
    let provider = ServiceCollection::new()
        .add_dbcontext_keyed("primary", |o| {
            o.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider");

    assert!(
        matches!(
            provider.get_keyed::<DbContext>("primary"),
            Err(RdiError::ScopedFromRootProvider { .. })
        ),
        "root shared resolution of a keyed Scoped context must be rejected"
    );

    let scope = provider.scope();
    assert!(
        scope.get_keyed::<DbContext>("primary").is_ok(),
        "keyed shared resolution must work inside a scope"
    );

    // Owned keyed resolution is still allowed from the root (caller-owned).
    let _owned: DbContext = provider
        .get_keyed_owned("primary")
        .expect("keyed owned context");
}

#[test]
fn keyed_resolution_accepts_string_and_service_keys() {
    // rust-dix 0.10 keyed APIs take `impl Into<ServiceKey>`. Keys registered by
    // `add_dbcontext_keyed("primary", ...)` are stored as String keys, so both
    // `String` and `ServiceKey::from("primary")` resolve them.
    let provider = ServiceCollection::new()
        .add_dbcontext_keyed("primary", |o| {
            o.use_sqlite_in_memory();
        })
        .build()
        .expect("build provider");

    let owned: DbContext = provider
        .get_keyed_owned(String::from("primary"))
        .expect("owned context with String key");
    drop(owned);

    let scope = provider.scope();
    let shared: Arc<DbContext> = scope
        .get_keyed(ServiceKey::from("primary"))
        .expect("shared context with ServiceKey");
    assert!(
        scope.is_registered_keyed::<DbContext>(ServiceKey::from("primary")),
        "keyed registration must be visible through ServiceKey"
    );
    drop(shared);

    // Documented 0.9 footgun: `ServiceKey::new("x")` is a `&str`-typed key and
    // is not equal to the String key stored by the string-keyed builder.
    assert!(
        scope
            .get_keyed::<DbContext>(ServiceKey::new("primary"))
            .is_err(),
        "&str-typed ServiceKey::new must not match the String registration"
    );
}

#[test]
fn scope_dispose_releases_scoped_instances() {
    // rust-dix 0.8+ owns disposal: the scope tracks every Scoped instance it
    // creates and releases them on drop. rust-ef registers no cleanup of its
    // own, so this test guards that contract for a service registered next to
    // `add_dbcontext`.
    struct Probe {
        dropped: Arc<std::sync::atomic::AtomicBool>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.dropped
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let provider = ServiceCollection::new()
        .add_dbcontext(|o| {
            o.use_sqlite_in_memory();
        })
        .scoped({
            let dropped = Arc::clone(&dropped);
            move |_| {
                Arc::new(Probe {
                    dropped: Arc::clone(&dropped),
                })
            }
        })
        .build()
        .expect("build provider");

    let scope = provider.scope();
    {
        let _ctx: Arc<DbContext> = scope.get().expect("get ctx");
        let _probe: Arc<Probe> = scope.get().expect("get probe");
    }
    assert!(
        !dropped.load(std::sync::atomic::Ordering::SeqCst),
        "scoped instance must stay alive while the scope is alive"
    );

    drop(scope); // Drop => dispose: releases scoped instances in reverse order.

    assert!(
        dropped.load(std::sync::atomic::Ordering::SeqCst),
        "scope disposal must drop Scoped instances created by rust-dix"
    );
}
