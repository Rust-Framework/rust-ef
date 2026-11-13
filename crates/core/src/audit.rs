//! Server-owned audit columns — auto-fill helper built on the EF-7 mutable
//! save hook.
//!
//! An entity opts in by implementing [`IAuditable`]; the host registers one
//! [`AuditInterceptor`] and lists the entity types it owns:
//!
//! ```rust,ignore
//! builder.add_interceptor(
//!     AuditInterceptor::new()
//!         .track::<Order>()
//!         .track::<Refund>(),
//! );
//! ```
//!
//! The interceptor writes audit columns in
//! [`ISaveChangesInterceptor::on_saving_mut`], **before** change detection, so
//! the values are committed by the save's own transaction and cannot be
//! overridden by the request body (the hook runs after request binding and
//! overwrites whatever was supplied).
//!
//! Clock and actor are ports ([`IClock`], [`IAuditActor`]) so tests can inject a
//! fixed clock/actor, and production can adapt request-scoped identity.
//!
//! [`ISaveChangesInterceptor::on_saving_mut`]: crate::interceptor::ISaveChangesInterceptor::on_saving_mut

use std::sync::Arc;

use crate::entity::EntityState;
use crate::error::EFResult;
use crate::interceptor::{ISaveChangesInterceptor, MutableSaveChangesContext};

/// An entity with server-owned audit columns.
///
/// `#[derive(EntityType)]` entities implement it once; only the columns the
/// table actually has need overriding (the other setters default to no-ops).
pub trait IAuditable {
    /// Sets the "last updated at" value (milliseconds since the UNIX epoch).
    fn set_updated_at(&mut self, now_ms: i64);

    /// Sets the "last updated by" value. Default: no-op (no actor column).
    fn set_updated_by(&mut self, _actor: Option<&str>) {}

    /// Sets the "created at" value on INSERT. Default: no-op.
    fn set_created_at(&mut self, _now_ms: i64) {}

    /// Sets the "created by" value on INSERT. Default: no-op.
    fn set_created_by(&mut self, _actor: Option<&str>) {}
}

/// Clock port used to stamp audit columns.
pub trait IClock: Send + Sync {
    /// Current time in milliseconds since the UNIX epoch.
    fn now_ms(&self) -> i64;
}

/// Default [`IClock`] backed by the system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl IClock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// Actor port used to stamp "created by"/"updated by".
pub trait IAuditActor: Send + Sync {
    /// Current actor (user id, service name, ...), or `None` for system work.
    fn current_actor(&self) -> Option<String>;
}

/// Default [`IAuditActor`] for contexts without a current actor.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAuditActor;

impl IAuditActor for NoAuditActor {
    fn current_actor(&self) -> Option<String> {
        None
    }
}

/// Type-erased "touch audit columns of `T`" function.
type AuditToucher =
    Arc<dyn Fn(&mut MutableSaveChangesContext<'_>, i64, Option<&str>) + Send + Sync>;

/// SaveChanges interceptor that fills [`IAuditable`] columns for every entity
/// type registered with [`track`](Self::track).
///
/// - INSERT (`Added`): `created_at`/`created_by` **and** `updated_at`/`updated_by`.
/// - UPDATE (any tracked existing entity): `updated_at`/`updated_by` only.
/// - Without a current actor ([`NoAuditActor`] / `None`): INSERT writes `NULL`
///   to the `*_by` columns (server-owned, caller values cannot win), while
///   UPDATE keeps the stored `updated_by` value instead of erasing it.
///
/// Registration is explicit per entity type (Rust has no specialization for
/// "all `T: IAuditable`"), one builder line per type, centralized in host setup.
pub struct AuditInterceptor {
    clock: Arc<dyn IClock>,
    actor: Arc<dyn IAuditActor>,
    touchers: Vec<AuditToucher>,
}

impl Default for AuditInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditInterceptor {
    /// Creates an interceptor using [`SystemClock`] and [`NoAuditActor`].
    pub fn new() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            actor: Arc::new(NoAuditActor),
            touchers: Vec::new(),
        }
    }

    /// Replaces the clock port (tests: fixed clock).
    pub fn with_clock(mut self, clock: Arc<dyn IClock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replaces the actor port (usually request-scoped identity).
    pub fn with_actor(mut self, actor: Arc<dyn IAuditActor>) -> Self {
        self.actor = actor;
        self
    }

    /// Registers an entity type whose audit columns this interceptor owns.
    pub fn track<T>(mut self) -> Self
    where
        T: IAuditable + 'static,
    {
        self.touchers.push(Arc::new(|ctx, now_ms, actor| {
            ctx.for_each_entity_mut::<T>(|state, entity| {
                if state == EntityState::Added {
                    // Server-owned values on insert: overwrite whatever the
                    // caller supplied, including `None` when there is no actor.
                    entity.set_created_at(now_ms);
                    entity.set_created_by(actor);
                    entity.set_updated_at(now_ms);
                    entity.set_updated_by(actor);
                    return;
                }
                entity.set_updated_at(now_ms);
                // Without a current actor (background job, migration) keep the
                // stored `updated_by` value: overwriting it with `None` would
                // erase who last changed the row.
                if actor.is_some() {
                    entity.set_updated_by(actor);
                }
            });
        }));
        self
    }
}

#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving_mut(&self, ctx: &mut MutableSaveChangesContext<'_>) -> EFResult<()> {
        let now_ms = self.clock.now_ms();
        let actor = self.actor.current_actor();
        for toucher in &self.touchers {
            toucher(ctx, now_ms, actor.as_deref());
        }
        Ok(())
    }
}
