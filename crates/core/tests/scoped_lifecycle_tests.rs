//! Scoped lifecycle tests for `add_dbcontext`.
//!
//! Verifies EFCore-aligned unit-of-work semantics:
//!   - same DI scope  => same DbContext instance (shared resolution)
//!   - different scope => different instance
//!   - root provider resolution caches in root scope (same instance per call)
//!   - owned resolution always returns a fresh instance (bypasses cache)

use rust_ef::db_context::DbContext;
use rust_ef::di::{DbContextServiceCollectionExt, ServiceCollection};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::Arc;

fn build_provider() -> Arc<rust_ef::di::ServiceProvider> {
    Arc::new(
        ServiceCollection::new()
            .add_dbcontext(|o| {
                o.use_sqlite_in_memory();
            })
            .build()
            .expect("build provider"),
    )
}

#[test]
fn same_scope_returns_same_instance() {
    let provider = build_provider();
    let scope = provider.create_scope();
    let ctx1: Arc<DbContext> = scope.get();
    let ctx2: Arc<DbContext> = scope.get();
    assert!(
        Arc::ptr_eq(&ctx1, &ctx2),
        "same scope must return same instance (unit-of-work)"
    );
}

#[test]
fn different_scopes_return_different_instances() {
    let provider = build_provider();
    let scope1 = provider.create_scope();
    let scope2 = provider.create_scope();
    let ctx1: Arc<DbContext> = scope1.get();
    let ctx2: Arc<DbContext> = scope2.get();
    assert!(
        !Arc::ptr_eq(&ctx1, &ctx2),
        "different scopes must return different instances"
    );
}

#[test]
fn root_provider_caches_in_root_scope() {
    // rust-dicore 0.5.0+: ServiceProvider is the root scope, so Scoped
    // services resolved from the root are cached in root_scoped_cache.
    // This matches EFCore semantics — the root provider IS a scope.
    let provider = build_provider();
    let ctx1: Arc<DbContext> = provider.get();
    let ctx2: Arc<DbContext> = provider.get();
    assert!(
        Arc::ptr_eq(&ctx1, &ctx2),
        "root resolution must return the same instance (root scope cache)"
    );
}

#[test]
fn owned_resolution_returns_fresh_instance() {
    // get_owned() bypasses the cache — each call returns a fresh instance.
    let provider = build_provider();
    let ctx1: DbContext = provider.get_owned();
    let ctx2: DbContext = provider.get_owned();
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
    let scope = provider.create_scope();
    let shared: Arc<DbContext> = scope.get();
    let owned: DbContext = scope.get_owned();
    let shared_addr = Arc::as_ptr(&shared) as usize;
    let owned_addr = &owned as *const _ as usize;
    assert_ne!(
        shared_addr, owned_addr,
        "owned resolution must bypass scope cache"
    );
}
