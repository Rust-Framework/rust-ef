//! Blog example — demonstrates core lref features.
//!
//! Shows: EntityType derivation, DbContext, Fluent API, LINQ-style
//! queries, change tracking, bulk operations, and the migration engine.

mod entities;
mod context;

use entities::{Blog, Post};
use context::BloggingContext;
use rust_ef::prelude::*;

#[tokio::main]
async fn main() -> Result<(), LrefError> {
    println!("=== Rust Entity Framework (rust-ef) Blog Example ===\n");

    let mut ctx = BloggingContext::new().await?;

    // 1. Add entities (EFCore: ctx.Blogs.Add(new Blog { ... }))
    println!("[1] Adding a new blog...");
    ctx.blogs.add(Blog {
        blog_id: 0,
        url: "https://devblogs.microsoft.com/dotnet".into(),
        rating: 5,
        posts: HasMany::new(),
    });
    println!("    Blog added (EntityState::Added).");

    println!("[2] Adding posts...");
    ctx.posts.add(Post {
        post_id: 0,
        title: "Announcing EF Core 9".into(),
        content: Some("EF Core 9 brings significant performance improvements...".into()),
        blog_id: 1,
        blog: BelongsTo::new(),
    });
    ctx.posts.add(Post {
        post_id: 0,
        title: "Getting Started with EF Core".into(),
        content: Some("This guide walks through your first EF Core app.".into()),
        blog_id: 1,
        blog: BelongsTo::new(),
    });
    println!("    Posts added.");

    // 2. SaveChanges (EFCore: await ctx.SaveChangesAsync())
    println!("\n[3] Saving changes...");
    let result = ctx.save_changes().await?;
    println!("    {}", result);

    // 3. Query with column-level filter (EFCore: .Where(b => b.Rating > 3))
    println!("\n[4] Query blogs with filter...");
    let blogs = ctx.blogs.query()
        .filter_column("rating", ">", 3i32)
        .order_by_column("url")
        .to_list()
        .await?;
    println!("    Found {} blog(s) with rating > 3.", blogs.len());

    // 4. Query with named include
    println!("\n[5] Query with eager loading (Include)...");
    let _blogs_with_posts = ctx.blogs.query()
        .include_named("posts")
        .to_list()
        .await?;
    println!("    Loaded blogs with posts.");

    // 5. Projection query
    println!("\n[6] Projection query (Select)...");
    let _summaries = ctx.posts.query()
        .select_columns(&["title", "blog_id"])
        .to_list()
        .await?;
    println!("    Projection executed.");

    // 6. Find by ID (EFCore: ctx.Blogs.FindAsync(1))
    println!("\n[7] Finding by primary key...");
    let _found = ctx.blogs.query()
        .filter_column("blog_id", "=", 1i32)
        .first_or_default()
        .await?;
    println!("    Find executed.");

    // 7. Count query
    println!("\n[8] Counting posts...");
    let count = ctx.posts.query().count().await?;
    println!("    Total posts: {}", count);

    // 8. Check change tracker
    println!("\n[9] Change tracker status...");
    if ctx.change_tracker().has_changes() {
        println!("    Pending changes detected.");
    } else {
        println!("    No pending changes.");
    }

    // 9. Bulk update (EFCore: .ExecuteUpdateAsync(s => s.SetProperty(...)))
    println!("\n[10] Bulk update (ExecuteUpdate)...");
    ctx.posts.query()
        .filter_column("title", "LIKE", "%EF Core%")
        .execute_update()
        .set_column("title", "Updated Title")
        .execute()
        .await?;
    println!("    Bulk update executed.");

    // 10. Bulk delete (EFCore: .ExecuteDeleteAsync())
    println!("\n[11] Bulk delete (ExecuteDelete)...");
    ctx.blogs.query()
        .filter_column("rating", "<", 1i32)
        .execute_delete()
        .await?;
    println!("    Bulk delete executed.");

    // 11. Migration engine demo
    println!("\n[12] Migration engine demo...");
    let engine = rust_ef::migration::MigrationEngine::new(rust_ef::migration::MigrationDialect::Postgres);
    let snapshot = engine.create_snapshot("initial", &[Blog::entity_meta(), Post::entity_meta()]);
    let migration = engine.generate("InitialCreate", &[Blog::entity_meta(), Post::entity_meta()], &None)?;
    println!("    Generated migration: {}", migration.id);
    println!("    Up SQL: {} chars", migration.up_sql.len());
    println!("    Snapshot: {} entity types", snapshot.entity_types.len());

    println!("\n=== Example Complete ===");
    println!("\nFeature Summary:");
    println!("  [x] EntityType derivation (#[derive(EntityType)])");
    println!("  [x] DbContext with ChangeTracker");
    println!("  [x] Fluent API (EntityTypeConfiguration<T>)");
    println!("  [x] LINQ-style queries (filter, order_by, include, select)");
    println!("  [x] Change tracking (EntityState, snapshots)");
    println!("  [x] SaveChanges (unit-of-work)");
    println!("  [x] Bulk operations (ExecuteUpdate, ExecuteDelete)");
    println!("  [x] PostgreSQL provider (pool + SQL dialect + type mapping)");
    println!("  [x] MySQL provider (sqlx pool + SQL dialect)");
    println!("  [x] SQLite provider (rusqlite + async wrapper)");
    println!("  [x] Migration engine (model diff + Up/Down SQL generation)");
    println!("  [x] CLI tool (migration add/apply/revert/list/script)");
    Ok(())
}
