//! DI integration — `AddDbContext` on `rust-dicore`.
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
//!     .add_dbcontext(|options| {
//!         options.use_sqlite("data source=app.db");
//!     })
//!     .build()
//!     .unwrap();
//!
//! let ctx: Arc<DbContext> = provider.get();
//! ```
//!
//! # Multiple databases (keyed)
//!
//! ```rust,ignore
//! let provider = ServiceCollection::new()
//!     .add_dbcontext_keyed("primary", |options| {
//!         options.use_postgres("host=primary/db");
//!     })
//!     .add_dbcontext_keyed("logs", |options| {
//!         options
//!             .use_sqlite("logs.db")
//!             .add_interceptor(AuditInterceptor);
//!     })
//!     .build()
//!     .unwrap();
//!
//! let primary: Arc<DbContext> = provider.get_keyed("primary");
//! let logs: Arc<DbContext> = provider.get_keyed("logs");
//! ```
//!
//! ## Scoped Lifetime
//!
//! `add_dbcontext` registers the context as **Scoped** — the same instance is
//! reused within a single DI `Scope`, and different scopes are isolated.
//! Resolving directly from the root `ServiceProvider` (without creating a
//! scope) degrades to a fresh instance per call (transient).
//!
//! Use `create_scope()` to create a scope for unit-of-work isolation:
//! ```rust,ignore
//! let scope = provider.create_scope();
//! let ctx: Arc<DbContext> = scope.get();
//! // Multiple `get` calls within `scope` return the same instance.
//! ```
//!
//! > **rust-webapp**: the HTTP pipeline automatically creates a scope per
//! > request. Handlers receive `Arc<DbContext>` pre-resolved — no
//! > manual scope management needed.

use crate::db_context::{DbContext, DbContextOptionsBuilder};
use std::sync::Arc;

/// Adds `add_dbcontext` and `add_dbcontext_keyed` to `rust_dicore::ServiceCollection`.
pub trait DbContextServiceCollectionExt {
    /// Registers a `DbContext` as **scoped** with default key.
    ///
    /// The closure receives a `DbContextOptionsBuilder` for provider
    /// configuration. Resolves as `Arc<DbContext>`.
    fn add_dbcontext(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;

    /// Registers a keyed `DbContext` as **scoped**.
    ///
    /// Use this when you need multiple database connections in the same
    /// application. Each key identifies a distinct `DbContext` instance
    /// with its own provider and interceptors.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// .add_dbcontext_keyed("logs", |options| {
    ///     options.use_sqlite("logs.db");
    /// })
    /// ```
    fn add_dbcontext_keyed(
        self,
        key: &str,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;
}

impl DbContextServiceCollectionExt for ::rust_dicore::ServiceCollection {
    fn add_dbcontext(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        let options = Arc::new(builder.build());

        self.scoped(move |_| {
            let ctx = DbContext::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<DbContext>
        })
    }

    fn add_dbcontext_keyed(
        self,
        key: &str,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        builder.context_key(key);
        let options = Arc::new(builder.build());

        self.keyed_scoped(key, move |_| {
            let ctx = DbContext::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<DbContext>
        })
    }
}

pub use rust_dicore::{ServiceCollection, ServiceProvider};
