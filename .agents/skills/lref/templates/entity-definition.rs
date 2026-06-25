// Template: lref entity definition
// Copy and modify for each entity type in your application.

use rust_ef::prelude::*;

// --- Primary entity ---

#[derive(Debug, Clone, EntityType)]
#[table("blogs")] // Database table name
pub struct Blog {
    // Primary key  - ?always include #[primary_key]
    #[primary_key]
    #[auto_increment] // SERIAL / AUTO_INCREMENT / AUTOINCREMENT
    pub blog_id: i32,

    // Required string with max length
    #[required]
    #[max_length(200)]
    pub url: String,

    // Plain column (no special attributes needed)
    pub rating: i32,

    // Collection navigation  - ?one Blog has many Posts
    // Note: HasMany<T> has NO trait bound  - ?it's a pure container
    #[navigation]
    pub posts: HasMany<Post>,
}

// --- Related entity ---

#[derive(Debug, Clone, EntityType)]
#[table("posts")]
pub struct Post {
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    // Optional field  - ?Option<T> is nullable in DB
    pub content: Option<String>,

    // Foreign key  - ?references Blog type
    #[foreign_key(Blog)]
    pub blog_id: i32,

    // Reference navigation  - ?Post belongs to one Blog
    #[navigation]
    pub blog: BelongsTo<Blog>,
}

// --- Entity with custom column names ---

#[derive(Debug, Clone, EntityType)]
#[table("users")]
pub struct User {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(100)]
    #[column("user_name")] // Column name differs from field name
    pub name: String,

    #[required]
    #[unique] // Unique index
    #[column("email_address")]
    pub email: String,

    #[not_mapped] // This field is NOT stored in DB
    pub temporary_token: Option<String>,
}
