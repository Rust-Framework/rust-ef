//! DI integration — `AddDbContext` on `rust-dicore`.
//!
//! Supports single-context (default) and multi-context (keyed) registration.
//! `DbContext` is registered as **Scoped** and can be resolved either as
//! `Arc<DbContext>` (shared within a scope) or as owned `DbContext` (fresh
//! instance, idiomatic `&mut self` access).
//!
//! # Recommended: owned resolution for handlers
//!
//! `DbContext` methods (`set::<T>()`, `save_changes()`, `detect_changes()`)
//! require `&mut self`. The idiomatic pattern is to **own** the context via
//! `get_owned()`, avoiding `Arc<Mutex>` and interior mutability entirely.
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
//! // Owned: fresh instance, direct &mut self access — no locks needed.
//! let mut ctx: DbContext = provider.get_owned();
//! ctx.set::<Blog>().add(blog);
//! ctx.save_changes().await?;
//! ```
//!
//! Handlers declare a bare `ctx: DbContext` field — `#[derive(Inject)]`
//! auto-detects owned fields and resolves them via `get_owned()`.
//! Use `#[inject(scoped)]` (not bare `#[inject]`, which defaults to Singleton)
//! to avoid captive dependency errors with the Scoped `DbContext`:
//! ```rust,ignore
//! #[derive(Inject)]
//! pub struct CreateBlogHandler {
//!     ctx: DbContext,            // bare T → owned resolution
//! }
//!
//! #[inject(scoped)]
//! #[async_trait]
//! impl IRequestHandler<CreateBlogRequest, BlogModel> for CreateBlogHandler {
//!     async fn handle(&mut self, req: CreateBlogRequest) -> Result<BlogModel> {
//!         self.ctx.set::<Blog>().add(blog);
//!         self.ctx.save_changes().await?;
//!         // ...
//!     }
//! }
//! ```
//!
//! # Shared resolution (within a scope)
//!
//! When multiple consumers in the same scope must share a single instance
//! (e.g. an `IHostedService` that seeds data before handlers run), resolve
//! as `Arc<DbContext>`:
//! ```rust,ignore
//! let scope = provider.create_scope();
//! let ctx: Arc<DbContext> = scope.get();
//! // Additional get() calls within this scope return the same instance.
//! ```
//!
//! > **Note**: `Arc<DbContext>` only provides `&self` access. Mutation
//! > requires `Arc::get_mut` (refcount == 1) or restructuring to owned
//! > resolution. Prefer `get_owned()` for any scope that needs `&mut self`.
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
//! // Owned keyed resolution (recommended for handlers):
//! let mut primary: DbContext = provider.get_keyed_owned("primary");
//! let mut logs: DbContext = provider.get_keyed_owned("logs");
//!
//! // Shared keyed resolution (within a scope):
//! // let primary: Arc<DbContext> = scope.get_keyed("primary");
//! ```
//!
//! ## Scoped Lifetime
//!
//! `add_dbcontext` registers the context as **Scoped**. Resolving via
//! `get()` shares the instance within a scope; resolving via `get_owned()`
//! bypasses the cache and returns a fresh instance each call (both are
//! isolated across scopes). Resolving directly from the root
//! `ServiceProvider` (without creating a scope) degrades to a fresh
//! instance per call (transient).
//!
//! > **rust-webapp**: the HTTP pipeline automatically creates a scope per
//! > request. Handlers are resolved via `get_owned::<Handler>()`, which
//! > owns a fresh `DbContext` — no manual scope management needed.

use crate::db_context::{DbContext, DbContextOptionsBuilder};
use std::sync::Arc;

/// Adds `add_dbcontext` and `add_dbcontext_keyed` to `rust_dicore::ServiceCollection`.
pub trait DbContextServiceCollectionExt {
    /// Registers a `DbContext` as **scoped** with default key.
    ///
    /// The closure receives a `DbContextOptionsBuilder` for provider
    /// configuration. Resolve via `get_owned::<DbContext>()` for idiomatic
    /// `&mut self` access, or `get::<DbContext>()` for shared `Arc` access
    /// within a scope.
    fn add_dbcontext(
        self,
        configure: impl FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    ) -> Self;

    /// Registers a keyed `DbContext` as **scoped**.
    ///
    /// Use this when you need multiple database connections in the same
    /// application. Each key identifies a distinct `DbContext` instance
    /// with its own provider and interceptors. Resolve via
    /// `get_keyed_owned::<DbContext>("key")` (owned) or
    /// `get_keyed::<DbContext>("key")` (shared `Arc`).
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
