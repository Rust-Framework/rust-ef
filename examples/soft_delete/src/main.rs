//! Soft Delete pattern with global query filters and SaveChanges interceptor.
//!
//! Demonstrates the recommended approach for soft delete in rust-ef:
//! 1. Entity with an `is_deleted: bool` column
//! 2. Global query filter (`is_deleted = false`) auto-applied to all queries
//! 3. Manual soft-delete: set `is_deleted = true` + mark entity as Modified
//! 4. `query_ignore_filters()` for admin queries that need to see deleted records
//! 5. `AuditInterceptor` logging save operations via the interceptor pipeline
//!
//! # Architecture note
//!
//! The current `ISaveChangesInterceptor` API receives `&SaveChangesContext`
//! (read-only entry views) and cannot mutate entity fields directly. Therefore,
//! soft delete is performed **manually** in application code rather than
//! automatically by an interceptor. The interceptor is used here for
//! **audit logging** — a complementary concern that interceptors CAN handle.

use async_trait::async_trait;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder, IDbContext};
use rust_ef::error::EFResult;
use rust_ef::interceptor::{ISaveChangesInterceptor, SaveChangesContext, SaveChangesResultContext};
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef::query::{BoolExpr, FilterCondition};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

/// Article entity with soft-delete support.
///
/// The `is_deleted` flag marks records as soft-deleted instead of physically
/// removing them. A global query filter ensures `is_deleted = false` is
/// automatically appended to all queries.
#[derive(Debug, Clone, EntityType)]
#[table("articles")]
struct Article {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    #[required]
    #[max_length(2000)]
    pub body: String,

    /// Soft-delete flag. `false` = active, `true` = soft-deleted.
    pub is_deleted: bool,
}

// ---------------------------------------------------------------------------
// Interceptor
// ---------------------------------------------------------------------------

/// Logs SaveChanges operations. In a real app, this would write to an audit
/// table or external logging system.
struct AuditInterceptor;

#[async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        if ctx.total_count() > 0 {
            println!(
                "  [Audit] Saving: +{} ~{} -{}",
                ctx.added_count(),
                ctx.modified_count(),
                ctx.deleted_count()
            );
        }
        Ok(())
    }

    async fn on_saved(
        &self,
        _ctx: &SaveChangesContext,
        result: &SaveChangesResultContext,
    ) -> EFResult<()> {
        println!("  [Audit] Saved: {} entity(es) affected", result.total());
        Ok(())
    }

    async fn on_save_failed(&self, _ctx: &SaveChangesContext, error: &rust_ef::error::EFError) {
        eprintln!("  [Audit] Save FAILED: {}", error);
    }
}

// ---------------------------------------------------------------------------
// Context setup
// ---------------------------------------------------------------------------

/// Creates an in-memory SQLite DbContext with:
/// - Auto-discovered Article entity
/// - Global query filter `is_deleted = false`
/// - AuditInterceptor registered
async fn create_context() -> EFResult<DbContext> {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    builder.add_interceptor(AuditInterceptor);
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options)?;

    // Register entity metadata
    ctx.discover_entities()?;

    // Global query filter: only show non-deleted articles
    let soft_delete_filter = BoolExpr::Filter(FilterCondition::with_values(
        "is_deleted",
        "=",
        vec![DbValue::Bool(false)],
    ));
    ctx.model().has_query_filter::<Article>(soft_delete_filter);

    // Create DbSet (injects the filter) and schema
    ctx.set::<Article>();
    ctx.ensure_created().await?;

    Ok(ctx)
}

// ---------------------------------------------------------------------------
// Soft-delete helper
// ---------------------------------------------------------------------------

/// Soft-deletes all tracked Article entries matching the given predicate.
///
/// Instead of `set.remove(&entity)` (which marks for physical DELETE),
/// we set `is_deleted = true` and let `detect_changes` mark the entity
/// as Modified. The subsequent `save_changes()` issues an UPDATE.
fn soft_delete_entries<F>(ctx: &mut DbContext, predicate: F)
where
    F: Fn(&Article) -> bool,
{
    let set = ctx.set::<Article>();
    for entry in set.tracked_entries_mut() {
        if predicate(entry) {
            entry.is_deleted = true;
        }
    }
    // detect_changes compares current snapshot vs original → marks as Modified
    ctx.set::<Article>().detect_changes();
}

// ---------------------------------------------------------------------------
// Demo
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> EFResult<()> {
    println!("=== Soft Delete Example ===\n");

    let mut ctx = create_context().await?;

    // -- Step 1: Insert articles --
    println!("[1] Inserting articles...");
    ctx.set::<Article>().add(Article {
        id: 0,
        title: "Getting Started with rust-ef".into(),
        body: "An introduction to the ORM...".into(),
        is_deleted: false,
    });
    ctx.set::<Article>().add(Article {
        id: 0,
        title: "Advanced Query Patterns".into(),
        body: "Explore linq! macro forms A/B/C...".into(),
        is_deleted: false,
    });
    ctx.set::<Article>().add(Article {
        id: 0,
        title: "Soft Delete Best Practices".into(),
        body: "Use global filters + manual flagging...".into(),
        is_deleted: false,
    });
    ctx.save_changes().await?;
    println!("    3 articles inserted.\n");

    // -- Step 2: Query active articles (filter auto-applied) --
    println!("[2] Querying active articles (is_deleted = false filter auto-applied)...");
    let active = ctx.set::<Article>().query().to_list().await?;
    println!("    Found {} active articles:", active.len());
    for a in &active {
        println!("      [{}] {} (is_deleted={})", a.id, a.title, a.is_deleted);
    }
    println!();

    // -- Step 3: Soft-delete one article --
    println!("[3] Soft-deleting 'Advanced Query Patterns'...");
    // `query().to_list()` (Step 2) returns Vec<T> but does NOT track entities.
    // `save_changes()` in Step 1 also cleared the tracker. To modify entities
    // via the change tracker, we must first load them as tracked (Unchanged).
    // `load_all()` queries (filter applied) and attaches each row.
    ctx.set::<Article>().load_all().await?;
    {
        let target_title = "Advanced Query Patterns".to_string();
        soft_delete_entries(&mut ctx, |a| a.title == target_title);
    }
    ctx.save_changes().await?;
    println!("    Soft-deleted 1 article (UPDATE, not DELETE).\n");

    // -- Step 4: Query again — soft-deleted article is hidden --
    println!("[4] Querying active articles after soft-delete...");
    let active_after = ctx.set::<Article>().query().to_list().await?;
    println!("    Found {} active articles:", active_after.len());
    for a in &active_after {
        println!("      [{}] {}", a.id, a.title);
    }
    println!();

    // -- Step 5: Admin query — bypass filter to see ALL articles --
    println!("[5] Admin query (query_ignore_filters) — see ALL articles...");
    let all = ctx
        .set::<Article>()
        .query_ignore_filters()
        .to_list()
        .await?;
    println!("    Found {} total articles:", all.len());
    for a in &all {
        let status = if a.is_deleted { "DELETED" } else { "active" };
        println!("      [{}] {} [{}]", a.id, a.title, status);
    }
    println!();

    // -- Step 6: Direct DB verification --
    println!("[6] Direct DB check — row still exists with is_deleted=1...");
    let provider = ctx.provider();
    let mut conn = provider.get_connection().await?;
    let rows = conn
        .query(
            "SELECT id, title, is_deleted FROM articles ORDER BY id",
            &[],
        )
        .await?;
    for row in &rows {
        let id = row.first().map(|s| s.as_str()).unwrap_or("?");
        let title = row.get(1).map(|s| s.as_str()).unwrap_or("?");
        let is_del = row.get(2).map(|s| s.as_str()).unwrap_or("?");
        println!(
            "      DB row: id={}, title='{}', is_deleted={}",
            id, title, is_del
        );
    }
    println!();

    println!("=== Example Complete ===");
    Ok(())
}
