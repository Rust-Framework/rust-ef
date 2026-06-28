//! Scoped lifecycle tests for `add_dbcontext`.
//!
//! Verifies EFCore-aligned unit-of-work semantics:
//!   - same DI scope  => same DbContext instance
//!   - different scope => different instance
//!   - root provider resolution degrades to transient (new instance each call)

use rust_ef::db_context::IDbContext;
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
    let ctx1: Arc<dyn IDbContext> = scope.get();
    let ctx2: Arc<dyn IDbContext> = scope.get();
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
    let ctx1: Arc<dyn IDbContext> = scope1.get();
    let ctx2: Arc<dyn IDbContext> = scope2.get();
    assert!(
        !Arc::ptr_eq(&ctx1, &ctx2),
        "different scopes must return different instances"
    );
}

#[test]
fn root_provider_resolution_degrades_to_transient() {
    let provider = build_provider();
    let ctx1: Arc<dyn IDbContext> = provider.get();
    let ctx2: Arc<dyn IDbContext> = provider.get();
    assert!(
        !Arc::ptr_eq(&ctx1, &ctx2),
        "root resolution must create a new instance each call (backward compatible)"
    );
}
