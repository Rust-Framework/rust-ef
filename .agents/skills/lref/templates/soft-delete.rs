// Template: Soft Delete pattern with global query filters.
//
// Covers the complete workflow:
// 1. Entity definition with is_deleted flag
// 2. Global query filter registration at startup
// 3. Soft-delete via detect_changes() (precise UPDATE)
// 4. Admin query to see all records (including deleted)
//
// See also: examples/soft_delete/src/main.rs for a runnable example.

use rust_ef::prelude::*;
use rust_ef::db_context::DbContext;

// ── 1. Entity Definition ──

#[derive(Debug, Clone, EntityType)]
#[table("articles")]
pub struct Article {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    pub content: String,

    /// Soft-delete flag. false = active, true = soft-deleted.
    /// The global query filter auto-appends `is_deleted = false` to all queries.
    pub is_deleted: bool,

    // Audit fields
    pub created_at: i64,
    pub updated_at: i64,
    pub updated_id: Option<i32>,
}

// ── 2. Register Global Query Filter at Startup ──

fn configure_soft_delete(ctx: &mut DbContext) {
    // Register once per entity type with soft-delete support.
    // All subsequent queries automatically exclude is_deleted = true records.
    ctx.model().entity::<Article>()
        .has_query_filter(linq!(filter |a: Article| !a.is_deleted));
}

// ── 3. Soft-Delete Handler ──

async fn soft_delete_article(
    ctx: &mut DbContext,
    id: i32,
    operator_id: Option<i32>,
    now: i64,
) -> Result<(), EFError> {
    // Load entity into tracker
    let mut article = ctx.set::<Article>().query().find(id).await?
        .ok_or(EFError::not_found("Article not found"))?;

    // Mark as deleted
    article.is_deleted = true;
    article.updated_at = now;
    article.updated_id = operator_id;

    // detect_changes: only changed fields appear in UPDATE SQL
    ctx.detect_changes();
    ctx.save_changes().await?;

    Ok(())
}

// ── 4. Admin Query (see all records) ──

async fn admin_list_all(ctx: &mut DbContext) -> Result<Vec<Article>, EFError> {
    // query_ignore_filters() bypasses the global query filter
    ctx.set::<Article>().query_ignore_filters().to_list().await
}

// ── 5. Normal Query (auto-filtered) ──

async fn list_active(ctx: &mut DbContext) -> Result<Vec<Article>, EFError> {
    // Global query filter auto-appends is_deleted = false
    ctx.set::<Article>().query().to_list().await
}

// ── 6. Bulk Soft-Delete (with load_all + detect_changes) ──

async fn soft_delete_by_title(
    ctx: &mut DbContext,
    title_pattern: &str,
) -> Result<usize, EFError> {
    // load_all: loads all matching rows into the tracker
    ctx.load_all::<Article>().await?;

    let mut count = 0;
    for entry in ctx.set::<Article>().tracked_entries_mut() {
        if entry.title.contains(title_pattern) {
            entry.is_deleted = true;
            count += 1;
        }
    }

    if count > 0 {
        ctx.detect_changes();
        ctx.save_changes().await?;
    }

    Ok(count)
}