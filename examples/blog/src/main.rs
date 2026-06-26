//! Blog example — demonstrates core rust-ef features with type-map `DbContext`.
//!
//! This example uses the v0.5.1+ auto-registration pattern:
//! - `#[derive(EntityType)]` automatically registers Blog/Post with `inventory`
//! - `#[entity_config(Blog)]` applies Fluent API overrides (renamed table & column)
//! - `ctx.discover_entities()` discovers all registered entities
//! - `ctx.ensure_created()` applies all configurations and creates the schema

mod context;
mod entities;

use context::create_blog_context;
use entities::{Blog, Post};
use rust_ef::db_context::IDbContext;
use rust_ef::linq;
use rust_ef::prelude::*;

#[tokio::main]
async fn main() -> Result<(), EFError> {
    println!("=== Rust Entity Framework (rust-ef) Blog Example ===\n");

    let mut ctx = create_blog_context().await?;

    println!("[0] Verify #[entity_config] overrides are applied...");
    {
        let metas = ctx.model().build();
        let blog_meta = metas
            .iter()
            .find(|m| m.type_name.contains("Blog"))
            .expect("Blog should be discovered");
        println!(
            "    Blog table: {} (renamed from 'blogs' by BlogConfig)",
            blog_meta.table_name
        );
        let url_prop = blog_meta
            .properties
            .iter()
            .find(|p| p.field_name.as_ref() == "url")
            .expect("url property should exist");
        println!(
            "    url column: {} (renamed from 'url' by BlogConfig), max_length: {:?}",
            url_prop.column_name, url_prop.max_length
        );
        assert_eq!(blog_meta.table_name.as_ref(), "blogs_renamed");
        assert_eq!(url_prop.column_name.as_ref(), "blog_url");
        assert_eq!(url_prop.max_length, Some(500));
        println!("    All overrides verified.");
    }

    println!("[1] Adding a new blog...");
    ctx.set::<Blog>().add(Blog {
        blog_id: 0,
        url: "https://devblogs.microsoft.com/dotnet".into(),
        rating: 5,
        posts: HasMany::new(),
    });

    println!("[2] Saving blog...");
    let result = ctx.save_changes().await?;
    println!("    {result}");

    let blogs = ctx.set::<Blog>().query().to_list().await?;
    let blog_id = blogs.first().map(|b| b.blog_id).unwrap_or(1);

    println!("[3] Adding posts for blog_id={blog_id}...");
    ctx.set::<Post>().add(Post {
        post_id: 0,
        title: "Announcing EF Core 9".into(),
        content: Some("EF Core 9 brings significant performance improvements...".into()),
        blog_id,
        blog: BelongsTo::new(),
    });
    ctx.set::<Post>().add(Post {
        post_id: 0,
        title: "Getting Started with EF Core".into(),
        content: Some("This guide walks through your first EF Core app.".into()),
        blog_id,
        blog: BelongsTo::new(),
    });
    ctx.save_changes().await?;

    println!("[4] Query blogs with linq! filter...");
    let filtered = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3)
        .to_list()
        .await?;
    println!("    Found {} blog(s) with rating > 3.", filtered.len());

    println!("[5] Eager load posts...");
    let _with_posts = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;
    println!("    Include executed.");

    println!("[6] Count posts...");
    let count = ctx.set::<Post>().query().count().await?;
    println!("    Total posts: {count}");

    println!("[7] Migration snapshot demo...");
    let engine =
        rust_ef::migration::MigrationEngine::new(rust_ef::migration::MigrationDialect::Sqlite);
    let metas = ctx.model().build();
    let migration = engine.generate("InitialCreate", &metas, &None)?;
    println!(
        "    Generated migration SQL ({} chars) using configured metas.",
        migration.up_sql.len()
    );

    println!("\n=== Example Complete ===");
    Ok(())
}
