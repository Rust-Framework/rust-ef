//! # Rust Entity Framework (lref)
//!
//! An EFCore-inspired ORM for Rust, bringing the familiar
//! DbContext/DbSet/EntityType patterns to the Rust ecosystem.
//!
//! ## Example
//!
//! ```rust,ignore
//! use lref::prelude::*;
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

pub mod entity;
pub mod metadata;
pub mod db_context;
pub mod db_set;
pub mod query;
pub mod model_builder;
pub mod tracking;
pub mod relations;
pub mod migration;
pub mod provider;
pub mod error;
pub mod change_executor;
pub mod cache;

/// Re-exports of the most commonly used types.
pub mod prelude {
    pub use crate::entity::EntityType;
    pub use crate::entity::EntityState;
    pub use crate::entity::FromRow;
    pub use crate::entity::GetKeyValues;
    pub use crate::entity::EntitySnapshot;
    pub use crate::metadata::EntityTypeMeta;
    pub use crate::metadata::PropertyMeta;
    pub use crate::metadata::NavigationMeta;
    pub use crate::db_context::DbContext;
    pub use crate::db_context::SaveChangesResult;
    pub use crate::tracking::ChangeTracker;
    pub use crate::db_set::DbSet;
    pub use crate::relations::{BelongsTo, HasMany, HasOne, DeleteBehavior};
    pub use crate::error::LrefError;
    pub use crate::model_builder::{ModelBuilder, EntityTypeBuilder, EntityTypeConfiguration, PropertyBuilder};
    pub use crate::provider::DatabaseProvider;
    pub use crate::provider::DbValue;
    pub use lref_macros::EntityType;
    pub use lref_macros::column;
    pub use crate::save_changes_all;
}
