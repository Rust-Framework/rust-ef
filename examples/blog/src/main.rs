//! Blog example — demonstrates core rust-ef features with type-map `DbContext`.

mod context;
mod entities;

use context::create_blog_context;
use entities::{Blog, Post};
use rust_ef::db_context::IDbContext;
use rust_ef::linq;
use rust_ef::prelude::*;

#[tokio::main]
async fn main() -> Result<(), EfError> {
    println!("=== Rust Entity Framework (rust-ef) Blog Example ===\n");

    let mut ctx = create_blog_context().await?;

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
    let _with_posts = ctx
        .set::<Blog>()
        .query()
        .include_named("posts")
        .to_list()
        .await?;
    println!("    Include executed.");

    println!("[6] Count posts...");
    let count = ctx.set::<Post>().query().count().await?;
    println!("    Total posts: {count}");

    println!("[7] Migration snapshot demo...");
    let engine = rust_ef::migration::MigrationEngine::new(rust_ef::migration::MigrationDialect::Sqlite);
    let migration = engine.generate(
        "InitialCreate",
        &[Blog::entity_meta(), Post::entity_meta()],
        &None,
    )?;
    println!("    Generated migration SQL ({} chars).", migration.up_sql.len());

    println!("\n=== Example Complete ===");
    Ok(())
}
