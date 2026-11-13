//! Minimal audit auto-fill built on the EF-7 mutable save hook:
//! `IAuditable` + `AuditInterceptor` + injectable `IClock`/`IAuditActor`.

use std::sync::{Arc, Mutex};

use rust_ef::audit::{AuditInterceptor, IAuditActor, IAuditable, IClock};
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("audit_orders")]
struct AuditOrder {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
    created_at: i64,
    updated_at: i64,
    created_by: Option<String>,
    updated_by: Option<String>,
}

impl IAuditable for AuditOrder {
    fn set_updated_at(&mut self, now_ms: i64) {
        self.updated_at = now_ms;
    }

    fn set_updated_by(&mut self, actor: Option<&str>) {
        self.updated_by = actor.map(str::to_string);
    }

    fn set_created_at(&mut self, now_ms: i64) {
        self.created_at = now_ms;
    }

    fn set_created_by(&mut self, actor: Option<&str>) {
        self.created_by = actor.map(str::to_string);
    }
}

/// `IAuditable` but deliberately NOT registered with `.track::<T>()`.
#[derive(Debug, Clone, EntityType)]
#[table("audit_untracked")]
struct AuditUntracked {
    #[primary_key]
    #[auto_increment]
    id: i32,
    updated_at: i64,
}

impl IAuditable for AuditUntracked {
    fn set_updated_at(&mut self, now_ms: i64) {
        self.updated_at = now_ms;
    }
}

#[derive(Default)]
struct FakeClock {
    now_ms: Mutex<i64>,
}

impl IClock for FakeClock {
    fn now_ms(&self) -> i64 {
        *self.now_ms.lock().unwrap()
    }
}

#[derive(Default)]
struct FakeActor {
    actor: Mutex<Option<String>>,
}

impl IAuditActor for FakeActor {
    fn current_actor(&self) -> Option<String> {
        self.actor.lock().unwrap().clone()
    }
}

struct Ports {
    clock: Arc<FakeClock>,
    actor: Arc<FakeActor>,
}

fn make_ctx() -> (DbContext, Ports) {
    let clock = Arc::new(FakeClock::default());
    let actor = Arc::new(FakeActor::default());
    let interceptor = AuditInterceptor::new()
        .with_clock(Arc::clone(&clock) as Arc<dyn IClock>)
        .with_actor(Arc::clone(&actor) as Arc<dyn IAuditActor>)
        .track::<AuditOrder>();

    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    builder.add_interceptor(interceptor);
    let options = builder.build();
    let ctx = DbContext::from_options(&options).expect("context");
    (ctx, Ports { clock, actor })
}

fn new_order(name: &str) -> AuditOrder {
    AuditOrder {
        id: 0,
        name: name.into(),
        // Deliberately client-supplied values: the interceptor must overwrite.
        created_at: -1,
        updated_at: -1,
        created_by: Some("client".into()),
        updated_by: Some("client".into()),
    }
}

async fn load_first_order(ctx: &mut DbContext) -> AuditOrder {
    ctx.set::<AuditOrder>()
        .query()
        .first()
        .await
        .expect("saved order")
}

#[tokio::test]
async fn insert_initializes_created_and_updated_stamps() {
    let (mut ctx, ports) = make_ctx();
    ctx.set::<AuditOrder>();
    ctx.ensure_created().await.unwrap();

    *ports.clock.now_ms.lock().unwrap() = 1_700_000_000_000;
    *ports.actor.actor.lock().unwrap() = Some("user-1".into());

    ctx.add::<AuditOrder>(new_order("alpha"));
    ctx.save_changes().await.expect("insert");

    let order = load_first_order(&mut ctx).await;
    assert_eq!(order.created_at, 1_700_000_000_000);
    assert_eq!(order.updated_at, 1_700_000_000_000);
    // Server-owned values overwrite the ones supplied by the caller.
    assert_eq!(order.created_by.as_deref(), Some("user-1"));
    assert_eq!(order.updated_by.as_deref(), Some("user-1"));
}

#[tokio::test]
async fn update_touches_updated_stamps_and_keeps_created() {
    let (mut ctx, ports) = make_ctx();
    ctx.set::<AuditOrder>();
    ctx.ensure_created().await.unwrap();

    *ports.clock.now_ms.lock().unwrap() = 1_000;
    *ports.actor.actor.lock().unwrap() = Some("creator".into());
    ctx.add::<AuditOrder>(new_order("alpha"));
    ctx.save_changes().await.expect("insert");

    ctx.clear_tracking();
    let loaded = load_first_order(&mut ctx).await;
    ctx.attach::<AuditOrder>(loaded);
    ctx.set::<AuditOrder>()
        .tracked_entries_mut()
        .next()
        .unwrap()
        .name = "alpha-renamed".into();

    *ports.clock.now_ms.lock().unwrap() = 2_000;
    *ports.actor.actor.lock().unwrap() = Some("editor".into());
    ctx.save_changes().await.expect("update");

    let order = load_first_order(&mut ctx).await;
    assert_eq!(order.name, "alpha-renamed");
    assert_eq!(order.created_at, 1_000, "created_at is insert-only");
    assert_eq!(order.created_by.as_deref(), Some("creator"));
    assert_eq!(order.updated_at, 2_000);
    assert_eq!(order.updated_by.as_deref(), Some("editor"));
}

#[tokio::test]
async fn only_tracked_entity_types_are_touched() {
    let (mut ctx, ports) = make_ctx();
    ctx.set::<AuditOrder>();
    ctx.set::<AuditUntracked>();
    ctx.ensure_created().await.unwrap();

    *ports.clock.now_ms.lock().unwrap() = 7_000;
    *ports.actor.actor.lock().unwrap() = Some("system".into());

    ctx.add::<AuditOrder>(new_order("alpha"));
    ctx.add::<AuditUntracked>(AuditUntracked {
        id: 0,
        updated_at: -1,
    });
    ctx.save_changes().await.expect("insert");

    let untracked = ctx
        .set::<AuditUntracked>()
        .query()
        .first()
        .await
        .expect("untracked row");
    assert_eq!(
        untracked.updated_at, -1,
        "entity types not registered with track() must not be touched"
    );
}

#[tokio::test]
async fn system_update_without_actor_preserves_updated_by() {
    let (mut ctx, ports) = make_ctx();
    ctx.set::<AuditOrder>();
    ctx.ensure_created().await.unwrap();

    *ports.clock.now_ms.lock().unwrap() = 1_000;
    *ports.actor.actor.lock().unwrap() = Some("creator".into());
    ctx.add::<AuditOrder>(new_order("alpha"));
    ctx.save_changes().await.expect("insert");

    ctx.clear_tracking();
    let loaded = load_first_order(&mut ctx).await;
    ctx.attach::<AuditOrder>(loaded);
    ctx.set::<AuditOrder>()
        .tracked_entries_mut()
        .next()
        .unwrap()
        .name = "alpha-system".into();

    // Background/system work: there is no current actor any more.
    *ports.actor.actor.lock().unwrap() = None;
    *ports.clock.now_ms.lock().unwrap() = 3_000;
    ctx.save_changes().await.expect("system update");

    let order = load_first_order(&mut ctx).await;
    assert_eq!(order.name, "alpha-system");
    assert_eq!(order.updated_at, 3_000, "updated_at is still stamped");
    assert_eq!(
        order.updated_by.as_deref(),
        Some("creator"),
        "a system update without an actor must not erase updated_by"
    );
    assert_eq!(order.created_by.as_deref(), Some("creator"));
}

#[tokio::test]
async fn insert_without_actor_clears_caller_supplied_audit_values() {
    let (mut ctx, ports) = make_ctx();
    ctx.set::<AuditOrder>();
    ctx.ensure_created().await.unwrap();

    *ports.clock.now_ms.lock().unwrap() = 4_000;
    *ports.actor.actor.lock().unwrap() = None;
    // `new_order` deliberately carries client-supplied created_by/updated_by.
    ctx.add::<AuditOrder>(new_order("alpha"));
    ctx.save_changes().await.expect("insert");

    let order = load_first_order(&mut ctx).await;
    assert_eq!(order.created_at, 4_000);
    assert_eq!(order.updated_at, 4_000);
    assert_eq!(
        order.created_by, None,
        "insert without an actor clears caller-supplied created_by (server-owned)"
    );
    assert_eq!(order.updated_by, None);
}
