//! DI integration — EFCore-style `AddDbContext<T>` on top of `lrdi`.
//!
//! Provides extension traits that add `add_dbcontext` / `resolve_dbcontext`
//! to `lrdi::ServiceCollection` / `lrdi::ServiceProvider`.
//!
//! # Example
//!
//! ```rust,ignore
//! use lrdi::ServiceCollection;
//! use lref::di::*;
//! use lref_provider_sqlite::DbContextOptionsBuilderExt as _;
//!
//! let provider = ServiceCollection::new()
//!     .add_dbcontext::<BloggingContext>(|options| {
//!         options.use_sqlite("data source=my.db3");
//!     })
//!     .build()
//!     .unwrap();
//!
//! // Resolve with explicit resolver (rebuilds context fresh each time)
//! let resolver = provider.as_resolver();
//! let mut ctx = provider.resolve_dbcontext::<BloggingContext>(&resolver).unwrap();
//! ctx.save_changes().await?;
//! ```

use crate::db_context::{DbContextOptions, DbContextOptionsBuilder, IDbContext};
use crate::error::LrefResult;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Extension trait on lrdi::ServiceCollection
// ---------------------------------------------------------------------------

/// Extension trait that adds `add_dbcontext<T>` to `lrdi::ServiceCollection`.
///
/// Implements the EFCore `AddDbContext<T>` pattern: stores `DbContextOptions`
/// as a singleton so they can be retrieved later when resolving the context.
///
/// # Example
///
/// ```rust,ignore
/// use lrdi::ServiceCollection;
/// use lref::di::DbContextServiceCollectionExt;
/// use lref_provider_sqlite::DbContextOptionsBuilderExt as _;
///
/// let provider = ServiceCollection::new()
///     .add_dbcontext::<BloggingContext>(|options| {
///         options.use_sqlite("data source=my.db3");
///     })
///     .build()
///     .unwrap();
/// ```
pub trait DbContextServiceCollectionExt {
    /// Registers a DbContext type with provider-specific configuration.
    ///
    /// The closure receives a `DbContextOptionsBuilder` that can be extended
    /// with provider-specific methods (e.g. `use_sqlite()`).
    fn add_dbcontext<T, F>(self, configure: F) -> Self
    where
        T: IDbContext + 'static,
        F: FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static;
}

impl DbContextServiceCollectionExt for ::lrdi::ServiceCollection {
    fn add_dbcontext<T, F>(self, configure: F) -> Self
    where
        T: IDbContext + 'static,
        F: FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static,
    {
        let mut builder = DbContextOptionsBuilder::new();
        configure(&mut builder);
        let options = builder.build();

        // Store options as a singleton — resolve_dbcontext reads them back
        // and calls T::from_options(&options, &resolver) to create the context.
        let opts = Arc::new(options);
        self.singleton(move |_| Arc::clone(&opts))
    }
}

// ---------------------------------------------------------------------------
// Extension trait on lrdi::ServiceProvider
// ---------------------------------------------------------------------------

/// Extension trait that adds `resolve_dbcontext<T>` to `lrdi::ServiceProvider`.
///
/// Creates a fresh context instance each call (transient semantics) using
/// the stored `DbContextOptions` and the provided `IServiceResolver`.
pub trait DbContextServiceProviderExt {
    /// Resolves and constructs a DbContext.
    ///
    /// `resolver` is obtained via `provider.as_resolver()` or from a
    /// lrdi factory closure's `|resolver|` parameter.
    fn resolve_dbcontext<T: IDbContext + 'static>(
        &self,
        resolver: &dyn lrdi::IServiceResolver,
    ) -> Option<LrefResult<T>>;
}

impl DbContextServiceProviderExt for ::lrdi::ServiceProvider {
    fn resolve_dbcontext<T: IDbContext + 'static>(
        &self,
        resolver: &dyn lrdi::IServiceResolver,
    ) -> Option<LrefResult<T>> {
        let options: Arc<DbContextOptions> = self.get();
        Some(T::from_options(&options, resolver))
    }
}

// ---------------------------------------------------------------------------
// Re-export lrdi types for convenience
// ---------------------------------------------------------------------------

pub use lrdi::{ServiceCollection, ServiceProvider};
