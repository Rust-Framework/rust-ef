// Template: DbContext usage  - ?type-map pattern, no entity-specific fields.
//
// DbContext stores entity sets in a HashMap<TypeId, Box<dyn Any>>.
// Access via ctx.set::<Entity>()  - ?lazy-creates DbSet on first call.
// save_changes() auto-discovers all entity types via SetOps dispatchers.

use rust_ef::prelude::*;
use rust_ef::db_context::{DbContext, DbContextOptions};
use rust_ef_sqlite::SqliteProvider; // or postgres / mysql
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), EFError> {
    // --- 1. Create provider and options ---
    let provider = Arc::new(SqliteProvider::new("data source=app.db")?);

    let options = DbContextOptions::default();
    // For DI-based construction, use provider_options instead (see di-setup.rs)

    // --- 2. Create context ---
    let mut ctx = DbContext::from_options(&options)?;
    // OR for manual provider setup:
    // let mut ctx = DbContext { ... }; // internal fields are not public

    // --- 3. Run migration (CREATE TABLE) ---
    let engine = rust_ef::migration::MigrationEngine::new(
        rust_ef::migration::MigrationDialect::Sqlite
    );
    let metas = vec![Blog::entity_meta(), Post::entity_meta()];
    let migration = engine.generate("InitialCreate", &metas, &None)?;
    provider.execute_migration_command(&migration.up_sql).await?;

    // --- 4. Use entity sets ---
    ctx.set::<Blog>().add(Blog {
        blog_id: 0,
        url: "https://example.com".into(),
        rating: 5,
        posts: HasMany::new(),
    });

    ctx.set::<Post>().add(Post {
        post_id: 0,
        title: "Hello World".into(),
        content: Some("First post content".into()),
        blog_id: 1,
        blog: BelongsTo::new(),
    });

    // --- 5. Save (auto-discovers all entity types) ---
    let result = ctx.save_changes().await?;
    println!("Saved: {}", result);

    // --- 6. Query ---
    let posts = ctx.set::<Post>().query()
        .filter_column("title", "LIKE", "%Hello%")
        .order_by_column("title")
        .to_list().await?;
    println!("Found {} posts", posts.len());

    Ok(())
}

// NOTE: The entity definitions (Blog, Post) are in a separate file.
// See templates/entity-definition.rs for the complete entity pattern.
