// Template: Web handler CRUD patterns for rust-ef with Arc<Mutex<DbContext>>.
//
// KEY RULES:
// 1. ONE lock acquisition per request — hold across the entire write flow
// 2. After save_changes(), auto_increment IDs are populated on the entity
// 3. Re-query by PRIMARY KEY (not slug/email) when you need navigation includes
// 4. Use detect_changes() for precise UPDATE SQL (not update() which marks all fields)
// 5. Use global query filters for is_deleted instead of repeating in every query

use std::sync::Arc;
use tokio::sync::Mutex;
use rust_ef::prelude::*;
use rust_ef::db_context::DbContext;

// ── Handler struct (DI-injectable) ──

#[derive(Inject)]
pub struct BlogHandler {
    ctx: Arc<Mutex<DbContext>>,
}

// ── CREATE ──

#[inject]
#[async_trait]
impl IRequestHandler<CreateBlogRequest, BlogModel> for BlogHandler {
    async fn handle(&self, req: CreateBlogRequest) -> Result<BlogModel> {
        // ONE lock for the entire write flow
        let mut ctx = self.ctx.lock().await;

        // 1. Check uniqueness (within the lock — no TOCTOU race)
        let exists = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == req.slug)
            .first_or_default().await?;
        if exists.is_some() {
            return Err("Slug already exists");
        }

        // 2. Insert
        let mut blog = req.to_entity(uid, now);
        ctx.set::<Blog>().add(blog);
        ctx.save_changes().await?;
        // blog.id is now populated with the auto_increment value

        // 3. Optional: re-query with navigation includes
        //    Only needed if the response requires navigation data.
        //    Re-query by PRIMARY KEY — not by slug/email.
        let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or("Blog vanished after insert")?;

        Ok(saved.to_model())
    }
}

// ── UPDATE ──

#[inject]
#[async_trait]
impl IRequestHandler<UpdateBlogRequest, BlogModel> for BlogHandler {
    async fn handle(&self, req: UpdateBlogRequest) -> Result<BlogModel> {
        let mut ctx = self.ctx.lock().await; // ONE lock

        // 1. Load existing entity
        let mut blog = ctx.set::<Blog>().query().find(req.id).await?
            .ok_or(Error::NotFound)?;

        // 2. Apply changes
        req.apply_to(&mut blog, uid, now);

        // 3. detect_changes: only changed fields → more precise UPDATE SQL
        ctx.set::<Blog>().detect_changes();
        ctx.save_changes().await?;

        // 4. Re-query with includes (by primary key)
        let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
        ).first_or_default().await?
            .ok_or("Blog not found after update")?;

        Ok(saved.to_model())
    }
}

// ── DELETE (Soft) ──

#[inject]
#[async_trait]
impl IRequestHandler<DeleteBlogRequest, String> for BlogHandler {
    async fn handle(&self, req: DeleteBlogRequest) -> Result<String> {
        let mut ctx = self.ctx.lock().await; // ONE lock

        let mut blog = ctx.set::<Blog>().query().find(req.id).await?
            .ok_or(Error::NotFound)?;

        blog.is_deleted = true;
        blog.updated_at = now;
        // Global query filter (is_deleted = false) will now exclude this record

        ctx.set::<Blog>().detect_changes();
        ctx.save_changes().await?;

        Ok(format!("Deleted blog {}", req.id))
    }
}

// ── LIST (Read) ──

#[inject]
#[async_trait]
impl IRequestHandler<ListBlogsRequest, Vec<BlogModel>> for BlogHandler {
    async fn handle(&self, _: ListBlogsRequest) -> Result<Vec<BlogModel>> {
        let mut ctx = self.ctx.lock().await;

        // With global query filter: is_deleted = false is auto-appended
        let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0;
            include b.category;
            order_by b.published_at desc;
        ).to_list().await?;

        Ok(blogs.into_iter().map(BlogModel::from).collect())
    }
}