//! DI integration — `AddDbContext<T>` on `lrdi`, interface-oriented.
//!
//! # Example
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
//! // Interface-oriented resolution
//! let ctx: Arc<dyn IDbContext> = provider.get();
//! ctx.save_changes().await?;
//! ```

use crate::db_context::{DbContext, DbContextOptions, DbContextOptionsBuilder, IDbContext};
use std::sync::Arc;

/// Adds `add_dbcontext<T>` to `lrdi::ServiceCollection`.
///
/// The closure receives a `DbContextOptionsBuilder` for provider configuration.
/// Resolves as `dyn IDbContext` — business code depends on the interface.
pub trait DbContextServiceCollectionExt {
    fn add_dbcontext<T, F>(self, configure: F) -> Self
    where
        T: IDbContext + FromDbContextOptions + 'static,
        F: FnOnce(&mut DbContextOptionsBuilder) + Send + Sync + 'static;
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
