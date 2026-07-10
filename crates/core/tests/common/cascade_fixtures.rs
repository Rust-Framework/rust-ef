//! Cascade test entity definitions shared across cascade_save_*.rs test files.

use rust_ef::prelude::*;

// ── One-to-many entities ──
#[derive(Debug, Clone, EntityType)]
#[table("cascade_blogs")]
pub struct CascadeBlog {
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,
    #[required]
    pub url: String,
    #[navigation]
    pub posts: HasMany<CascadePost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_posts")]
pub struct CascadePost {
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,
    #[required]
    pub title: String,
    #[foreign_key(CascadeBlog)]
    pub blog_id: i32,
    #[navigation]
    pub blog: BelongsTo<CascadeBlog>,
}

// ── Self-referential entity ──
#[derive(Debug, Clone, EntityType)]
#[table("cascade_categories")]
pub struct CascadeCategory {
    #[primary_key]
    #[auto_increment]
    pub category_id: i32,
    #[required]
    pub name: String,
    #[foreign_key(CascadeCategory)]
    pub parent_id: i32,
    #[navigation]
    pub children: HasMany<CascadeCategory>,
}

// ── Many-to-many entities ──
#[derive(Debug, Clone, EntityType)]
#[table("cascade_students")]
pub struct CascadeStudent {
    #[primary_key]
    #[auto_increment]
    pub student_id: i32,
    #[required]
    pub name: String,
    #[navigation]
    pub courses: HasMany<CascadeCourse, CascadeEnrollment>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_courses")]
pub struct CascadeCourse {
    #[primary_key]
    #[auto_increment]
    pub course_id: i32,
    #[required]
    pub title: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_enrollments")]
pub struct CascadeEnrollment {
    #[primary_key]
    #[auto_increment]
    pub enrollment_id: i32,
    #[foreign_key(CascadeStudent)]
    pub student_id: i32,
    #[foreign_key(CascadeCourse)]
    pub course_id: i32,
}

// ── Nested cascade entities (Blog → Post → Comment) ──
#[derive(Debug, Clone, EntityType)]
#[table("cascade_nest_blogs")]
pub struct CascadeNestBlog {
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,
    #[required]
    pub url: String,
    #[navigation]
    pub posts: HasMany<CascadeNestPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_nest_posts")]
pub struct CascadeNestPost {
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,
    #[required]
    pub title: String,
    #[foreign_key(CascadeNestBlog)]
    pub blog_id: i32,
    #[navigation]
    pub blog: BelongsTo<CascadeNestBlog>,
    #[navigation]
    pub comments: HasMany<CascadeNestComment>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_nest_comments")]
pub struct CascadeNestComment {
    #[primary_key]
    #[auto_increment]
    pub comment_id: i32,
    #[required]
    pub text: String,
    #[foreign_key(CascadeNestPost)]
    pub post_id: i32,
}

// ── SetNull cascade entities (nullable FK + #[on_delete(SetNull)]) ──
#[derive(Debug, Clone, EntityType)]
#[table("cascade_optional_blogs")]
pub struct CascadeOptionalBlog {
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,
    #[required]
    pub url: String,
    #[navigation]
    #[on_delete(SetNull)]
    pub posts: HasMany<CascadeOptionalPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_optional_posts")]
pub struct CascadeOptionalPost {
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,
    #[required]
    pub title: String,
    #[foreign_key(CascadeOptionalBlog)]
    pub blog_id: Option<i32>,
    #[navigation]
    pub blog: BelongsTo<CascadeOptionalBlog>,
}
