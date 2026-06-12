//! DI integration — `AddDbContext<T>` on `lrdi`, interface-oriented.
//!
//! Supports single-context (default) and multi-context (keyed) registration.
//!
//! # Single database (recommended)
//!
//! ```rust,ignore
//! use lrdi::ServiceCollection;
//! use lref::di::*;
//! use lref::db_context::DbContext;
//! use lref_provider_sqlite::DbContextOptionsBuilderExt as _;
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

use crate::db_context::{DbContext, DbContextOptions, DbContextOptionsBuilder, IDbContext};
use std::sync::Arc;

/// Adds `add_dbcontext`, `add_dbcontext_keyed`, and
/// `add_dbcontext_from_options` to `lrdi::ServiceCollection`.
pub trait DbContextServiceCollectionExt {
    /// Registers a `DbContext` as transient with default key.
    ///
    /// The closure receives a `DbContextOptionsBuilder` for provider
    /// configuration. Resolves as `Arc<dyn IDbContext>`.
    fn add_dbcontext<T, F>(self, configure: F) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
        F: FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static;

    /// Registers a keyed `DbContext` as transient.
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
    fn add_dbcontext_keyed<T, F>(self, key: &str, configure: F) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
        F: FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static;

    /// Registers a `DbContext` from a pre-built `DbContextOptions`.
    ///
    /// Use this when you want full control over option construction
    /// or when sharing the same options across multiple registrations.
    fn add_dbcontext_from_options<T>(self, options: DbContextOptions) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static;
}

impl DbContextServiceCollectionExt for ::lrdi::ServiceCollection {
    fn add_dbcontext<T, F>(self, configure: F) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
        F: FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    {
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        let options = Arc::new(builder.build());

        self.transient(move |_| {
            let ctx = T::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<dyn IDbContext>
        })
    }

    fn add_dbcontext_keyed<T, F>(self, key: &str, configure: F) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
        F: FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    {
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        let options = Arc::new(builder.build());

        self.keyed_transient(key, move |_| {
            let ctx = T::from_options(&options).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<dyn IDbContext>
        })
    }

    fn add_dbcontext_from_options<T>(self, options: DbContextOptions) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
    {
        let opts = Arc::new(options);

        self.transient(move |_| {
            let ctx = T::from_options(&opts).expect("Failed to create DbContext");
            Arc::new(ctx) as Arc<dyn IDbContext>
        })
    }
}

/// Trait for types that can be constructed from `DbContextOptions`.
pub trait FromDbContextOptions: IDbContext + Sized {
    fn from_options(options: &DbContextOptions) -> crate::error::LrefResult<Self>;
}

impl FromDbContextOptions for DbContext {
    fn from_options(options: &DbContextOptions) -> crate::error::LrefResult<Self> {
        DbContext::from_options(options)
    }
}

pub use lrdi::{ServiceCollection, ServiceProvider};
