//! DI integration  - ?`AddDbContext<T>` on `rust-dicore`, interface-oriented.
//!
//! Supports single-context (default) and multi-context (keyed) registration.
//!
//! # Single database (recommended)
//!
//! ```rust,ignore
//! use rust_dicore::ServiceCollection;
//! use rust_ef::di::*;
//! use rust_ef::db_context::DbContext;
//! use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
//!
//! let provider = ServiceCollection::new()
//!     .add_dbcontext::<DbContext>(|options| {
//!         options.use_sqlite("data source=app.db");
//!     })
//!     .build()
//!     .unwrap();
//!
//! let ctx: Arc<dyn IDbContext> = provider.get();
//! ```
//!
//! # Multiple databases (keyed)
//!
//! ```rust,ignore
//! let provider = ServiceCollection::new()
//!     .add_dbcontext_keyed::<DbContext>("primary", |options| {
//!         options.use_postgres("host=primary/db");
//!     })
//!     .add_dbcontext_keyed::<DbContext>("logs", |options| {
//!         options
//!             .use_sqlite("logs.db")
//!             .add_interceptor(AuditInterceptor);
//!     })
//!     .build()
//!     .unwrap();
//!
//! let primary: Arc<dyn IDbContext> = provider.get_keyed("primary");
//! let logs: Arc<dyn IDbContext> = provider.get_keyed("logs");
//! ```
//!
//! ## Scoped Lifetime
//!
//! `add_dbcontext` registers the context as **Scoped** — the same instance is
//! reused within a single DI `Scope`, and different scopes are isolated.
//! Resolving directly from the root `ServiceProvider` (without creating a
//! scope) degrades to a fresh instance per call (transient).
//!
//! Use `DbContextScopeExt::create_dbcontext_scope()` to create a scope:
//! ```rust,ignore
//! let scope = provider.create_dbcontext_scope();
//! let ctx: Arc<dyn IDbContext> = scope.get();
//! // Multiple `get` calls within `scope` return the same instance.
//! ```
//!
//! This mirrors EFCore's `ServiceLifetime.Scoped` and is essential for
//! unit-of-work isolation: each request/operation gets its own `DbContext`,
//! preventing cross-request tracking pollution.

use crate::db_context::{DbContext, DbContextOptions, DbContextOptionsBuilder, IDbContext};
use std::sync::Arc;

/// Adds `add_dbcontext`, `add_dbcontext_keyed`, and
/// `add_dbcontext_from_options` to `rust_dicore::ServiceCollection`.
pub trait DbContextServiceCollectionExt {
    /// Registers a `DbContext` as **scoped** with default key.
    ///
    /// The closure receives a `DbContextOptionsBuilder` for provider
    /// configuration. Resolves as `Arc<dyn IDbContext>`.
    ///
    /// Scoped lifetime mirrors EFCore's `ServiceLifetime.Scoped`: the same
    /// instance is returned within a single DI scope (unit-of-work), and a
    /// fresh instance per scope. Resolving directly from the root
    /// `ServiceProvider` (without `create_scope()`) degrades to transient
    /// behaviour, which is safe and backward compatible.
    fn add_dbcontext<T>(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static;

    /// Registers a keyed `DbContext` as **scoped**.
    ///
    /// Use this when you need multiple database connections in the same
    /// application. Each key identifies a distinct `DbContext` instance
    /// with its own provider and interceptors.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// .add_dbcontext_keyed::<DbContext>("logs", |options| {
    ///     options.use_sqlite("logs.db");
    /// })
    /// ```
    fn add_dbcontext_keyed<T>(
        self,
        key: &str,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static;

    /// Registers a `DbContext` from a pre-built `DbContextOptions`.
    ///
    /// Use this when you want full control over option construction
    /// or when sharing the same options across multiple registrations.
    fn add_dbcontext_from_options<T>(self, options: DbContextOptions) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static;
}

impl DbContextServiceCollectionExt for ::rust_dicore::ServiceCollection {
    fn add_dbcontext<T>(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
    {
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        let options = Arc::new(builder.build());

        self.scoped(move |_| {
            let ctx = T::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<dyn IDbContext>
        })
    }

    fn add_dbcontext_keyed<T>(
        self,
        key: &str,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
    {
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        let options = Arc::new(builder.build());

        self.keyed_scoped(key, move |_| {
            let ctx = T::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<dyn IDbContext>
        })
    }

    fn add_dbcontext_from_options<T>(self, options: DbContextOptions) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
    {
        let opts = Arc::new(options);

        self.scoped(move |_| {
            let ctx = T::from_options(&opts).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<dyn IDbContext>
        })
    }
}

/// Trait for types that can be constructed from `DbContextOptions`.
pub trait FromDbContextOptions: IDbContext + Sized {
    fn from_options(options: &DbContextOptions) -> crate::error::EFResult<Self>;
}

impl FromDbContextOptions for DbContext {
    fn from_options(options: &DbContextOptions) -> crate::error::EFResult<Self> {
        DbContext::from_options(options)
    }
}

/// Convenience trait for creating a scoped DbContext resolution scope.
///
/// Scoped-lifetime `DbContext` instances registered via `add_dbcontext`
/// are cached within a `Scope` (unit-of-work isolation). Two resolutions
/// of `dyn IDbContext` from the same `Scope` return the same instance;
/// different scopes return different instances.
///
/// # Example
///
/// ```rust,ignore
/// let provider = Arc::new(
///     ServiceCollection::new()
///         .add_dbcontext::<DbContext>(|o| o.use_sqlite("app.db"))
///         .build()?,
/// );
///
/// let scope = provider.create_dbcontext_scope();
/// let ctx1: Arc<dyn IDbContext> = scope.get();
/// let ctx2: Arc<dyn IDbContext> = scope.get();
/// assert!(Arc::ptr_eq(&ctx1, &ctx2)); // same unit-of-work
/// ```
pub trait DbContextScopeExt {
    /// Creates a new DI scope. Scoped DbContext instances resolved from
    /// this `Scope` are cached within it (unit-of-work isolation).
    fn create_dbcontext_scope(self: &Arc<Self>) -> rust_dicore::Scope;
}

impl DbContextScopeExt for ServiceProvider {
    fn create_dbcontext_scope(self: &Arc<Self>) -> rust_dicore::Scope {
        self.create_scope()
    }
}

pub use rust_dicore::{ServiceCollection, ServiceProvider};
