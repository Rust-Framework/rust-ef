//! Audit logging pattern with SaveChanges interceptors and timestamp management.
//!
//! Demonstrates:
//! 1. `Document` entity with `created_at` / `updated_at` timestamps (i64 epoch)
//! 2. Manual timestamp filling before `save_changes()` (interceptor can't mutate)
//! 3. `AuditInterceptor` capturing save events into a shared buffer
//! 4. `AuditLog` table populated after each save by draining the buffer
//! 5. Separation of concerns: interceptor observes, application code persists
//!
//! # Architecture note
//!
//! `ISaveChangesInterceptor::on_saving` receives `&SaveChangesContext` whose
//! `entries()` yield `EntityEntryView { type_id, type_name, state }` — no entity
//! reference, no field values, no DB connection. Therefore:
//! - **Timestamps** are set manually in application code (like soft delete)
//! - **Audit records** are captured by the interceptor into an `Arc<Mutex<…>>`
//!   buffer, then drained and written to the `audit_log` table by the application
//!   after `save_changes()` returns

use async_trait::async_trait;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::entity::EntityState;
use rust_ef::error::EFResult;
use rust_ef::interceptor::{ISaveChangesInterceptor, SaveChangesContext, SaveChangesResultContext};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

/// Document entity with audit timestamps.
#[derive(Debug, Clone, EntityType)]
#[table("documents")]
struct Document {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    #[required]
    #[max_length(5000)]
    pub body: String,

    /// Unix epoch seconds — set on insert, never changed.
    pub created_at: i64,

    /// Unix epoch seconds — set on insert, refreshed on every update.
    pub updated_at: i64,
}

/// Audit log entry recording each save operation.
///
/// This table is the "change history" recommended pattern: one row per
/// entity-state transition, capturing who/what/when at a coarse granularity.
/// Field-level diffing requires extending the interceptor API (future work).
#[derive(Debug, Clone, EntityType)]
#[table("audit_log")]
struct AuditLog {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    /// Entity type name (e.g. "Document").
    #[required]
    #[max_length(100)]
    pub entity_type: String,

    /// Action: "INSERT", "UPDATE", or "DELETE".
    #[required]
    #[max_length(20)]
    pub action: String,

    /// Unix epoch seconds when the save was initiated.
    pub occurred_at: i64,

    /// Number of entities affected in this save batch.
    pub affected: i32,
}

// ---------------------------------------------------------------------------
// Audit interceptor
// ---------------------------------------------------------------------------

/// A single audit event captured by the interceptor.
#[derive(Debug, Clone)]
struct AuditEvent {
    entity_type: String,
    action: &'static str,
    occurred_at: i64,
}

/// Interceptor that captures save events into a shared buffer.
///
/// The buffer is `Arc<Mutex<Vec<AuditEvent>>>` so the application code can
/// drain it after `save_changes()` and persist records to the `audit_log` table.
struct AuditInterceptor {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

#[async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        let now = now_epoch();
        let mut buf = self.events.lock().expect("audit buffer poisoned");
        for entry in ctx.entries() {
            // Skip AuditLog itself to avoid feedback loop when flush_audit_log
            // calls save_changes() to persist audit records.
            if entry.type_name == "AuditLog" {
                continue;
            }
            let action = match entry.state {
                EntityState::Added => "INSERT",
                EntityState::Modified => "UPDATE",
                EntityState::Deleted => "DELETE",
                _ => continue,
            };
            buf.push(AuditEvent {
                entity_type: entry.type_name.clone(),
                action,
                occurred_at: now,
            });
        }
        if !buf.is_empty() {
            println!("  [Audit] Captured {} event(s) before save", buf.len());
        }
        Ok(())
    }

    async fn on_saved(
        &self,
        _ctx: &SaveChangesContext,
        result: &SaveChangesResultContext,
    ) -> EFResult<()> {
        println!(
            "  [Audit] Save completed: {} entity(es) affected",
            result.total()
        );
        Ok(())
    }

    async fn on_save_failed(&self, _ctx: &SaveChangesContext, error: &rust_ef::error::EFError) {
        eprintln!("  [Audit] Save FAILED: {}", error);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns the current Unix epoch time in seconds.
fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Stamps `updated_at` on tracked Document entries matching a predicate.
///
/// This is the manual timestamp pattern: since the interceptor cannot mutate
/// entities, the application fills timestamps before calling `save_changes()`.
/// Only matching entries are stamped to avoid unnecessary UPDATEs on unchanged rows.
fn stamp_updated_at<F>(ctx: &mut DbContext, predicate: F)
where
    F: Fn(&Document) -> bool,
{
    let now = now_epoch();
    for doc in ctx.set::<Document>().tracked_entries_mut() {
        if predicate(doc) {
            doc.updated_at = now;
        }
    }
    ctx.set::<Document>().detect_changes();
}

/// Drains captured audit events and writes them to the `audit_log` table.
///
/// Called after each `save_changes()`. Each save batch produces one `AuditLog`
/// row summarising the affected entities.
async fn flush_audit_log(
    ctx: &mut DbContext,
    events: &Arc<Mutex<Vec<AuditEvent>>>,
) -> EFResult<()> {
    let drained: Vec<AuditEvent> = events
        .lock()
        .expect("audit buffer poisoned")
        .drain(..)
        .collect();
    if drained.is_empty() {
        return Ok(());
    }
    // Group by (entity_type, action) and count; use the first event's timestamp
    let mut summary: std::collections::HashMap<(String, &'static str), (i32, i64)> =
        std::collections::HashMap::new();
    for ev in &drained {
        let entry = summary
            .entry((ev.entity_type.clone(), ev.action))
            .or_insert((0, ev.occurred_at));
        entry.0 += 1;
    }
    for ((entity_type, action), (count, ts)) in summary {
        ctx.set::<AuditLog>().add(AuditLog {
            id: 0,
            entity_type,
            action: action.to_string(),
            occurred_at: ts,
            affected: count,
        });
    }
    ctx.save_changes().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Context setup
// ---------------------------------------------------------------------------

async fn create_context(audit_buffer: Arc<Mutex<Vec<AuditEvent>>>) -> EFResult<DbContext> {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    builder.add_interceptor(AuditInterceptor {
        events: audit_buffer,
    });
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options)?;
    ctx.discover_entities()?;
    ctx.set::<Document>();
    ctx.set::<AuditLog>();
    ctx.ensure_created().await?;
    Ok(ctx)
}

// ---------------------------------------------------------------------------
// Demo
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> EFResult<()> {
    println!("=== Audit Example ===\n");

    let audit_buffer: Arc<Mutex<Vec<AuditEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let mut ctx = create_context(audit_buffer.clone()).await?;

    // -- Step 1: Insert documents with timestamps --
    println!("[1] Inserting documents with created_at / updated_at...");
    let now = now_epoch();
    ctx.set::<Document>().add(Document {
        id: 0,
        title: "Design Doc: Query Pipeline".into(),
        body: "Outlines the linq! macro expansion...".into(),
        created_at: now,
        updated_at: now,
    });
    ctx.set::<Document>().add(Document {
        id: 0,
        title: "Design Doc: Migration Engine".into(),
        body: "Covers snapshot diffing and SQL generation...".into(),
        created_at: now,
        updated_at: now,
    });
    ctx.save_changes().await?;
    println!("    2 documents inserted.\n");

    // Drain audit events → write to audit_log table
    flush_audit_log(&mut ctx, &audit_buffer).await?;
    println!("    Audit log entries written.\n");

    // -- Step 2: Update one document (load → modify → stamp → save) --
    println!("[2] Updating 'Design Doc: Query Pipeline'...");
    ctx.set::<Document>().load_all().await?;
    {
        let target = "Design Doc: Query Pipeline".to_string();
        for doc in ctx.set::<Document>().tracked_entries_mut() {
            if doc.title == target {
                doc.body = "Revised: now includes Form C subquery support...".into();
            }
        }
    }
    // Stamp updated_at only on the modified entry (predicate matches by title)
    stamp_updated_at(&mut ctx, |d| d.title == "Design Doc: Query Pipeline");
    ctx.save_changes().await?;
    println!("    1 document updated.\n");

    flush_audit_log(&mut ctx, &audit_buffer).await?;
    println!("    Audit log entries written.\n");

    // -- Step 3: Verify documents --
    println!("[3] Querying documents...");
    let docs = ctx.set::<Document>().query().to_list().await?;
    for d in &docs {
        println!(
            "      [{}] {} (created_at={}, updated_at={})",
            d.id, d.title, d.created_at, d.updated_at
        );
    }
    // updated_at on the modified doc should differ from created_at
    let modified = docs
        .iter()
        .find(|d| d.title == "Design Doc: Query Pipeline")
        .expect("doc exists");
    assert!(
        modified.updated_at >= modified.created_at,
        "updated_at should be >= created_at"
    );
    println!();

    // -- Step 4: Query audit log --
    println!("[4] Querying audit log...");
    let logs = ctx.set::<AuditLog>().query().to_list().await?;
    println!("    Found {} audit log entries:", logs.len());
    for l in &logs {
        println!(
            "      [{}] {} {} (affected={}, at={})",
            l.id, l.entity_type, l.action, l.affected, l.occurred_at
        );
    }
    println!();

    // -- Step 5: Direct DB verification --
    println!("[5] Direct DB check — audit_log rows...");
    let provider = ctx.provider();
    let mut conn = provider.get_connection().await?;
    let rows = conn
        .query(
            "SELECT id, entity_type, action, affected FROM audit_log ORDER BY id",
            &[],
        )
        .await?;
    for row in &rows {
        let id = row
            .first()
            .map(|s| format!("{}", s))
            .unwrap_or_else(|| "?".into());
        let et = row
            .get(1)
            .map(|s| format!("{}", s))
            .unwrap_or_else(|| "?".into());
        let act = row
            .get(2)
            .map(|s| format!("{}", s))
            .unwrap_or_else(|| "?".into());
        let aff = row
            .get(3)
            .map(|s| format!("{}", s))
            .unwrap_or_else(|| "?".into());
        println!(
            "      DB: id={}, entity='{}', action='{}', affected={}",
            id, et, act, aff
        );
    }
    println!();

    println!("=== Example Complete ===");
    Ok(())
}
