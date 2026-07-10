// Template: Web handler CRUD patterns for rust-ef with owned DbContext.
//
// KEY RULES:
// 1. add_dbcontext registers as Scoped — each request gets its own DbContext instance
// 2. Handlers own DbContext directly — bare field marked #[inject(owned)] resolves via get_owned()
// 3. handle(&mut self) — set::<T>() and save_changes() require &mut self
// 4. After save_changes(), auto_increment IDs are populated on the entity
// 5. Re-query by PRIMARY KEY (not slug/email) when you need navigation includes
// 6. Use detect_changes() for precise UPDATE SQL (not update() which marks all fields)
// 7. Use global query filters for is_deleted instead of repeating in every query
// 8. Use step-by-step let bindings for readability and debugging

use rust_ef::prelude::*;
use rust_ef::db_context::DbContext;

// ── Handler struct (DI-injectable) ──

#[derive(Inject)]
pub struct BlogHandler {
    #[inject(owned)]
    ctx: DbContext,  // bare T + #[inject(owned)] → get_owned()
}

// ── CREATE ──

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<CreateBlogRequest, BlogModel> for BlogHandler {
    async fn handle(&mut self, req: CreateBlogRequest) -> Result<BlogModel> {
        // 1. Check uniqueness
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let exists = set.filter(expr).first_or_default().await?;
        if exists.is_some() {
            return Err("Slug already exists");
        }

        // 2. Insert
        let mut blog = req.to_entity(uid, now);
        self.ctx.add::<Blog>(blog);
        self.ctx.save_changes().await?;
        // blog.id is now populated with the auto_increment value

        // 3. Optional: re-query with navigation includes
        //    Only needed if the response requires navigation data.
        //    Re-query by PRIMARY KEY — not by slug/email.
        let saved = linq!(self.ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or("Blog vanished after insert")?;

        Ok(saved.to_model())
    }
}

// ── READ (list) ──

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<ListBlogRequest, Paginated<BlogModel>> for BlogHandler {
    async fn handle(&mut self, req: ListBlogRequest) -> Result<Paginated<BlogModel>> {
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.rating > 0);
        let blogs = set.filter(expr)
            .skip(req.page * req.size)
            .take(req.size)
            .to_list().await?;
        Ok(blogs.into_iter().map(|b| b.to_model()).collect())
    }
}

// ── READ (single) ──

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<GetBlogRequest, BlogModel> for BlogHandler {
    async fn handle(&mut self, req: GetBlogRequest) -> Result<BlogModel> {
        let query = self.ctx.set::<Blog>().query();
        let blog = query.find(req.id).await?
            .ok_or("Blog not found")?;
        Ok(blog.to_model())
    }
}

// ── UPDATE ──

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<UpdateBlogRequest, BlogModel> for BlogHandler {
    async fn handle(&mut self, req: UpdateBlogRequest) -> Result<BlogModel> {
        // 1. Load existing entity
        let query = self.ctx.set::<Blog>().query();
        let mut blog = query.find(req.id).await?
            .ok_or("Blog not found")?;

        // 2. Apply changes
        blog.title = req.title;
        blog.content = req.content;

        // 3. Save (detect_changes only marks actually changed fields)
        self.ctx.detect_changes();
        self.ctx.save_changes().await?;

        // 4. Re-query with navigation includes (by PRIMARY KEY)
        let saved = linq!(self.ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
        ).first_or_default().await?
            .ok_or("Blog not found after update")?;

        Ok(saved.to_model())
    }
}

// ── DELETE (soft delete) ──

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<DeleteBlogRequest, String> for BlogHandler {
    async fn handle(&mut self, req: DeleteBlogRequest) -> Result<String> {
        // 1. Load existing entity
        let query = self.ctx.set::<Blog>().query();
        let mut blog = query.find(req.id).await?
            .ok_or("Blog not found")?;

        // 2. Soft delete: mark + detect_changes + save
        blog.is_deleted = true;
        blog.updated_at = chrono::Utc::now().timestamp();
        self.ctx.detect_changes();
        self.ctx.save_changes().await?;

        Ok(format!("Deleted blog {}", req.id))
    }
}
