// Template: DbContext usage — type-map pattern, auto-discovery, no entity-specific fields.
//
// DbContext stores entity sets in a HashMap<TypeId, Box<dyn Any>>.
// Access via ctx.set::<Entity>() — lazy-creates DbSet on first call.
// save_changes() auto-discovers all entity types via SetOps dispatchers.
//
// IMPORTANT: #[derive(EntityType)] auto-registers entities at compile time.
// DbContext::from_options() automatically calls discover_entities().
// No manual entity_meta() collection needed.

use rust_ef::prelude::*;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::main]
async fn main() -> Result<(), EFError> {
    // --- 1. Build options with provider ---
    let mut options = DbContextOptionsBuilder::new();
    options.use_sqlite_in_memory();

    // --- 2. Create context (auto-discovers all #[derive(EntityType)] entities) ---
    let mut ctx = DbContext::from_options(&options.build())?;
    // discover_entities() is called automatically — no manual setup needed

    // --- 3. Global query filters (soft delete) — register once at startup ---
    ctx.model().entity::<Blog>()
        .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));

    // --- 4. Create schema ---
    ctx.ensure_created().await?;

    // --- 5. Use entity sets ---
    ctx.add::<Blog>(Blog {
        id: 0,
        slug: "hello-world".into(),
        title: "Hello World".into(),
        content: "First blog post".into(),
        tags: String::new(),
        category_id: 1,
        author_id: 1,
        published_at: 0,
        created_at: 0,
        updated_at: 0,
        created_id: None,
        updated_id: None,
        is_deleted: false,
        category: BelongsTo::new(),
        author: BelongsTo::new(),
        comments: HasMany::new(),
    });

    // --- 6. Save (auto-discovers all entity types) ---
    let result = ctx.save_changes().await?;
    println!("Saved: {}", result);

    // --- 7. Query with linq! (type-safe, recommended) ---
    let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3;
        include b.category;
        order_by b.created_at desc;
    ).to_list().await?;
    println!("Found {} blogs", blogs.len());

    // --- 8. Simple PK lookup with query().find(id) ---
    let blog = ctx.set::<Blog>().query().find(1).await?;
    println!("Blog: {:?}", blog);

    Ok(())
}