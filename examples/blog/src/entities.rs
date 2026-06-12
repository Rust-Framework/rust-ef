//! Entity type definitions -- Blog and Post.
//!
//! These demonstrate the core entity definition pattern with
//! `#[derive(EntityType)]` and attribute-based configuration.

use lref::prelude::*;

/// Represents a Blog -- analogous to EFCore's Blog entity.
///
/// # Attributes
/// - `#[derive(EntityType)]`: Auto-implements IEntityType trait
/// - `#[table("blogs")]`: Maps to the `blogs` table
/// - `#[primary_key]`: Primary key (EFCore: `[Key]`)
/// - `#[auto_increment]`: Auto-increment / identity column
/// - `#[required]`: NOT NULL constraint (EFCore: `[Required]`)
/// - `#[max_length(N)]`: Maximum length (EFCore: `[MaxLength]`)
/// - `#[navigation]`: Navigation property (relationship)
#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    /// Primary key (EFCore: `public int BlogId { get; set; }`)
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,

    /// Blog URL (EFCore: `public string Url { get; set; }`)
    #[required]
    #[max_length(200)]
    pub url: String,

    /// Blog rating (EFCore: `public int Rating { get; set; }`)
    #[allow(dead_code)]
    pub rating: i32,

    /// Collection navigation: Blog has many Posts
    /// (EFCore: `public ICollection<Post> Posts { get; set; }`)
    #[navigation]
    pub posts: HasMany<Post>,
}

/// Represents a Post -- analogous to EFCore's Post entity.
///
/// Demonstrates:
/// - Foreign key relationship (`#[foreign_key(Blog)]`)
/// - Optional field using `Option<String>` (EFCore: nullable string)
/// - BelongsTo navigation
#[derive(Debug, Clone, EntityType)]
#[table("posts")]
pub struct Post {
    /// Primary key
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,

    /// Post title (required)
    #[required]
    #[max_length(200)]
    pub title: String,

    /// Post content (optional -- Option<T> naturally maps to nullable)
    pub content: Option<String>,

    /// Foreign key to Blog (EFCore: `public int BlogId { get; set; }`)
    #[foreign_key(Blog)]
    pub blog_id: i32,

    /// Reference navigation: Post belongs to Blog
    /// (EFCore: `public Blog Blog { get; set; }`)
    #[navigation]
    pub blog: BelongsTo<Blog>,
}
