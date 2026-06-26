//! # Rust Entity Framework (rust-ef)
//!
//! An EFCore-inspired ORM for Rust, bringing the familiar
//! DbContext/DbSet/EntityType patterns to the Rust ecosystem.
//!
//! ## Example
//!
//! ```rust,ignore
//! use rust_ef::prelude::*;
//!
//! #[derive(EntityType)]
//! #[table("blogs")]
//! pub struct Blog {
//!     #[primary_key]
//!     #[auto_increment]
//!     pub blog_id: i32,
//!     #[required]
//!     #[max_length(200)]
//!     pub url: String,
//!     pub rating: i32,
//!     #[navigation]
//!     pub posts: HasMany<Post>,
//! }
//! ```

pub mod change_executor;
pub mod db_context;
pub mod db_set;
pub mod di;
pub mod entity;
pub mod error;
pub mod interceptor;
pub mod metadata;
pub mod migration;
pub mod model_builder;
pub mod navigation_loader;
pub mod provider;
pub mod query;
pub mod registration;
pub mod relations;
pub mod tracking;

pub use async_trait;

pub use inventory;

pub use rust_ef_macros::{column, entity_config, linq, EntityType};

/// Re-exports of the most commonly used types.
pub mod prelude {
    pub use crate::db_context::{
        DbContext, DbContextOptions, DbContextOptionsBuilder, IDbContext, SaveChangesResult,
    };
    pub use crate::db_set::{DbSet, IDbSet};
    pub use crate::di::{DbContextScopeExt, DbContextServiceCollectionExt};
    pub use crate::entity::{
        EntityState, IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter,
    };
    pub use crate::error::{EFError, EFResult};
    pub use crate::interceptor::{
        ISaveChangesInterceptor, SaveChangesContext, SaveChangesResultContext,
    };
    pub use crate::metadata::EntityTypeMeta;
    pub use crate::metadata::NavigationMeta;
    pub use crate::metadata::PropertyMeta;
    pub use crate::model_builder::{
        EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder, PropertyBuilder,
    };
    pub use crate::provider::DbValue;
    pub use crate::provider::IDatabaseProvider;
    pub use crate::query::BoolExpr;
    pub use crate::registration::{EntityConfigRegistration, EntityRegistration};
    pub use crate::relations::{BelongsTo, DeleteBehavior, HasMany, HasOne};
    pub use crate::tracking::ChangeTracker;
    pub use rust_ef_macros::column;
    pub use rust_ef_macros::entity_config;
    pub use rust_ef_macros::linq;
    pub use rust_ef_macros::EntityType;
}
