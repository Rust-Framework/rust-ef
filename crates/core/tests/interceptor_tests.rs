//! EF-7: mutable SaveChanges interceptor (`on_saving_mut`).
//!
//! The read-only `on_saving` snapshot cannot fill server-owned columns such as
//! `updated_at` / `updated_by`. `on_saving_mut` hands over the actual tracked
//! entities before change detection, so values written there are committed by
//! the same transaction as the rest of the save.

use std::sync::{Arc, Mutex};

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::interceptor::{
    ISaveChangesInterceptor, MutableSaveChangesContext, SaveChangesContext,
};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("ef7_audit_items")]
struct AuditItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
    created_at: String,
    updated_at: String,
    updated_by: Option<String>,
}

/// Fills `created_at` on insert and always overwrites `updated_at`/`updated_by`
/// from a test-controlled clock, recording what it saw.
struct AuditInterceptor {
    now: Arc<Mutex<String>>,
    order: Arc<Mutex<Vec<&'static str>>>,
    seen_states: Arc<Mutex<Vec<EntityState>>>,
    visited: Arc<Mutex<usize>>,
}

impl AuditInterceptor {
    fn new(now: &str) -> (Self, AuditControls) {
        let now = Arc::new(Mutex::new(now.to_string()));
        let order = Arc::new(Mutex::new(Vec::new()));
        let seen_states = Arc::new(Mutex::new(Vec::new()));
        let visited = Arc::new(Mutex::new(0));
        (
            Self {
                now: Arc::clone(&now),
                order: Arc::clone(&order),
                seen_states: Arc::clone(&seen_states),
                visited: Arc::clone(&visited),
            },
            AuditControls {
                now,
                order,
                seen_states,
                visited,
            },
        )
    }
}

/// Test-side handles to the interceptor's observations.
struct AuditControls {
    now: Arc<Mutex<String>>,
    order: Arc<Mutex<Vec<&'static str>>>,
    seen_states: Arc<Mutex<Vec<EntityState>>>,
    visited: Arc<Mutex<usize>>,
}

impl AuditControls {
    fn set_now(&self, value: &str) {
        *self.now.lock().unwrap() = value.to_string();
    }
}

#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving_mut(&self, ctx: &mut MutableSaveChangesContext<'_>) -> EFResult<()> {
        self.order.lock().unwrap().push("on_saving_mut");

        let now = self.now.lock().unwrap().clone();
        let mut states = Vec::new();
        let visited = ctx.for_each_entity_mut::<AuditItem>(|state, item| {
            states.push(state);
            if state == EntityState::Added {
                item.created_at = now.clone();
            }
            // Server-owned: overwrite whatever the caller/request supplied.
            item.updated_at = now.clone();
            item.updated_by = Some("system".into());
        });

        *self.visited.lock().unwrap() = visited;
        *self.seen_states.lock().unwrap() = states;
        Ok(())
    }

    async fn on_saving(&self, _ctx: &SaveChangesContext) -> EFResult<()> {
        self.order.lock().unwrap().push("on_saving");
        Ok(())
    }
}

fn make_ctx(interceptor: AuditInterceptor) -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    builder.add_interceptor(interceptor);
    let options = builder.build();
    DbContext::from_options(&options).expect("context")
}

fn new_item(name: &str) -> AuditItem {
    AuditItem {
        id: 0,
        name: name.into(),
        // Deliberately wrong: the server-owned hook must overwrite these.
        created_at: String::new(),
        updated_at: String::new(),
        updated_by: None,
    }
}

async fn load_first(ctx: &mut DbContext) -> AuditItem {
    ctx.set::<AuditItem>()
        .query()
        .first()
        .await
        .expect("saved row")
}

#[tokio::test]
async fn mutable_hook_fills_audit_columns_on_insert() {
    let (interceptor, controls) = AuditInterceptor::new("2026-11-13T00:00:00Z");
    let mut ctx = make_ctx(interceptor);
    ctx.set::<AuditItem>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<AuditItem>(new_item("alpha"));
    ctx.save_changes().await.expect("save");

    // The row as read back from the database carries the interceptor values.
    let item = load_first(&mut ctx).await;
    assert_eq!(item.created_at, "2026-11-13T00:00:00Z");
    assert_eq!(item.updated_at, "2026-11-13T00:00:00Z");
    assert_eq!(item.updated_by.as_deref(), Some("system"));

    assert_eq!(*controls.visited.lock().unwrap(), 1);
    assert_eq!(
        *controls.seen_states.lock().unwrap(),
        vec![EntityState::Added]
    );
}

#[tokio::test]
async fn mutable_hook_touches_update_and_preserves_created_at() {
    let (interceptor, controls) = AuditInterceptor::new("2026-11-13T00:00:00Z");
    let mut ctx = make_ctx(interceptor);
    ctx.set::<AuditItem>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<AuditItem>(new_item("alpha"));
    ctx.save_changes().await.expect("insert");

    // Start a fresh unit of work: read-modify-save on a DB-loaded entity.
    ctx.clear_tracking();
    let loaded = load_first(&mut ctx).await;
    ctx.attach::<AuditItem>(loaded);
    ctx.set::<AuditItem>()
        .tracked_entries_mut()
        .next()
        .unwrap()
        .name = "alpha-renamed".into();

    controls.set_now("2026-11-13T01:00:00Z");
    ctx.save_changes().await.expect("update");

    let item = load_first(&mut ctx).await;
    assert_eq!(item.name, "alpha-renamed");
    // Created timestamp is initialized once and never overwritten on update.
    assert_eq!(item.created_at, "2026-11-13T00:00:00Z");
    assert_eq!(item.updated_at, "2026-11-13T01:00:00Z");
    assert_eq!(item.updated_by.as_deref(), Some("system"));

    // The mutable hook runs before change detection, so the tracked entity is
    // still `Unchanged` when the hook sees it.
    assert_eq!(
        *controls.seen_states.lock().unwrap(),
        vec![EntityState::Unchanged]
    );
    assert_eq!(*controls.visited.lock().unwrap(), 1);
}

#[tokio::test]
async fn mutable_hook_runs_before_read_only_hook() {
    let (interceptor, controls) = AuditInterceptor::new("2026-11-13T00:00:00Z");
    let mut ctx = make_ctx(interceptor);
    ctx.set::<AuditItem>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<AuditItem>(new_item("alpha"));
    ctx.save_changes().await.expect("save");

    assert_eq!(
        *controls.order.lock().unwrap(),
        vec!["on_saving_mut", "on_saving"]
    );
}

/// An interceptor that aborts the save before any SQL runs.
struct RejectingInterceptor;

#[async_trait::async_trait]
impl ISaveChangesInterceptor for RejectingInterceptor {
    async fn on_saving_mut(&self, ctx: &mut MutableSaveChangesContext<'_>) -> EFResult<()> {
        let mut blocked = false;
        ctx.for_each_entity_mut::<AuditItem>(|_, item| {
            if item.name == "blocked" {
                blocked = true;
            }
        });
        if blocked {
            return Err(EFError::configuration("rejected by interceptor"));
        }
        Ok(())
    }
}

#[tokio::test]
async fn interceptor_error_aborts_save_before_sql() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    builder.add_interceptor(RejectingInterceptor);
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("context");
    ctx.set::<AuditItem>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<AuditItem>(new_item("blocked"));
    let err = ctx.save_changes().await.expect_err("save must abort");
    assert!(
        err.message().contains("rejected by interceptor"),
        "unexpected error: {err:?}"
    );

    // Pre-commit rejection: nothing reached the database.
    let rows = ctx.set::<AuditItem>().query().to_list().await.unwrap();
    assert!(
        rows.is_empty(),
        "no row may be written when the hook rejects"
    );
}

/// Read-only interceptors (implementing only `on_saving`) keep working: the
/// new mutable hook has a default no-op implementation.
struct ReadOnlyInterceptor {
    seen: Arc<Mutex<usize>>,
}

#[async_trait::async_trait]
impl ISaveChangesInterceptor for ReadOnlyInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        *self.seen.lock().unwrap() = ctx.total_count();
        Ok(())
    }
}

#[tokio::test]
async fn read_only_interceptor_still_supported() {
    let seen = Arc::new(Mutex::new(0));
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    builder.add_interceptor(ReadOnlyInterceptor {
        seen: Arc::clone(&seen),
    });
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("context");
    ctx.set::<AuditItem>();
    ctx.ensure_created().await.unwrap();

    ctx.add::<AuditItem>(new_item("alpha"));
    ctx.save_changes().await.expect("save");
    assert_eq!(*seen.lock().unwrap(), 1);
}
