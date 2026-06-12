//! DbContext trait, DbContextOptions, and ChangeTracker — the session / unit-of-work layer.
//!
//! ## Architecture
//!
//! `IDbContext` is object-safe — no `Sized`, no associated type, no generic methods.
//! This enables `dyn IDbContext` resolution from DI containers.
//!
//! Entity sets use a type-map: `ctx.set::<Blog>()` lazy-creates `DbSet<Blog>`.
//! `SetOps<T>` dispatchers enable `save_changes()` to iterate all entity types.
//!
//! ## Provider Factory
//!
//! `DbContextOptions` stores a `provider_factory` closure injected by the
//! provider extension methods (`use_sqlite`, `use_postgres`, `use_mysql`).
//! `AppDbContext::new(options)` calls this factory to create the provider.

use crate::change_executor::ChangeExecutor;
use crate::db_set::{DbSet, IDbSet};
use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues};
use crate::error::LrefResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{IAsyncConnection, IDatabaseProvider};
use crate::tracking::ChangeTracker;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// DbContextOptions / DbContextOptionsBuilder
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct DbContextOptions {
    pub(crate) connection_string: String,
    pub(crate) provider_tag: Option<String>,
    pub(crate) provider_factory:
        Option<Arc<dyn Fn(&str) -> LrefResult<Arc<dyn IDatabaseProvider>> + Send + Sync>>,
}

impl std::fmt::Debug for DbContextOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbContextOptions")
            .field("connection_string", &self.connection_string)
            .field("provider_tag", &self.provider_tag)
            .finish()
    }
}

impl DbContextOptions {
    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }
    pub fn provider_tag(&self) -> Option<&str> {
        self.provider_tag.as_deref()
    }
    pub fn create_provider(&self) -> LrefResult<Arc<dyn IDatabaseProvider>> {
        let factory = self.provider_factory.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider configured. Call use_sqlite / use_postgres / use_mysql first.".into(),
            )
        })?;
        factory(self.connection_string())
    }
}

impl Default for DbContextOptions {
    fn default() -> Self {
        Self {
            connection_string: String::new(),
            provider_tag: None,
            provider_factory: None,
        }
    }
}

pub struct DbContextOptionsBuilder {
    inner: DbContextOptions,
}

impl DbContextOptionsBuilder {
    pub fn new() -> Self {
        Self {
            inner: DbContextOptions::default(),
        }
    }
    pub fn connection_string(&mut self, cs: impl Into<String>) -> &mut Self {
        self.inner.connection_string = cs.into();
        self
    }
    pub fn set_provider(&mut self, tag: &str, cs: impl Into<String>) -> &mut Self {
        self.inner.provider_tag = Some(tag.to_string());
        self.inner.connection_string = cs.into();
        self
    }
    pub fn set_provider_factory(
        &mut self,
        tag: &str,
        cs: impl Into<String>,
        factory: Arc<dyn Fn(&str) -> LrefResult<Arc<dyn IDatabaseProvider>> + Send + Sync>,
    ) -> &mut Self {
        self.inner.provider_tag = Some(tag.to_string());
        self.inner.connection_string = cs.into();
        self.inner.provider_factory = Some(factory);
        self
    }
    pub fn build(self) -> DbContextOptions {
        self.inner
    }
}

impl Default for DbContextOptionsBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Type-erased set operations
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
trait ErasedSetOps: Send + Sync {
    async fn save(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
    ) -> LrefResult<(usize, usize, usize)>;
    fn clear(&self, raw_set: &mut (dyn Any + Send + Sync + 'static));
}

struct SetOps<E> {
    _phantom: std::marker::PhantomData<E>,
}
impl<E> SetOps<E> {
    fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<E> ErasedSetOps for SetOps<E>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow + Send + Sync + 'static,
{
    async fn save(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
    ) -> LrefResult<(usize, usize, usize)> {
        let db_set = raw_set
            .downcast_mut::<DbSet<E>>()
            .expect("SetOps type mismatch");
        save_one_set(conn, provider, db_set).await
    }
    fn clear(&self, raw_set: &mut (dyn Any + Send + Sync + 'static)) {
        if let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() {
            db_set.clear_entries();
        }
    }
}

// ---------------------------------------------------------------------------
// AppDbContext
// ---------------------------------------------------------------------------

pub struct AppDbContext {
    sets: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    savers: HashMap<TypeId, Box<dyn ErasedSetOps>>,
    change_tracker: ChangeTracker,
    provider: Arc<dyn IDatabaseProvider>,
}

impl AppDbContext {
    /// Creates the context from options (uses the provider factory stored in options).
    pub fn from_options(options: &DbContextOptions) -> LrefResult<Self> {
        let provider = options.create_provider()?;
        Ok(Self {
            sets: HashMap::new(),
            savers: HashMap::new(),
            change_tracker: ChangeTracker::new(),
            provider,
        })
    }

    pub fn set<T>(&mut self) -> &mut DbSet<T>
    where
        T: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.savers
            .entry(type_id)
            .or_insert_with(|| Box::new(SetOps::<T>::new()));
        self.sets.entry(type_id).or_insert_with(|| {
            let meta = T::entity_meta();
            Box::new(DbSet::<T>::with_provider(
                meta.table_name.as_ref(),
                Arc::clone(&self.provider),
            ))
        });
        self.sets
            .get_mut(&type_id)
            .and_then(|b| b.downcast_mut::<DbSet<T>>())
            .expect("DbSet type mismatch")
    }
}

// ---------------------------------------------------------------------------
// IDbContext — object-safe
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait IDbContext: Send + Sync {
    fn provider(&self) -> &dyn IDatabaseProvider;
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker;
    fn change_tracker(&self) -> &ChangeTracker;
    async fn save_changes(&mut self) -> LrefResult<SaveChangesResult>;

    async fn begin_transaction(&self) -> LrefResult<Box<dyn IAsyncConnection>> {
        let mut conn = self.provider().get_connection().await?;
        conn.begin_transaction().await?;
        Ok(conn)
    }
}

#[async_trait::async_trait]
pub trait IDbContextExt: IDbContext {
    async fn use_transaction<F, Fut, R>(&self, f: F) -> LrefResult<R>
    where
        F: FnOnce(&mut dyn IAsyncConnection) -> Fut + Send,
        Fut: Future<Output = LrefResult<R>> + Send,
        R: Send,
    {
        let mut conn = self.provider().get_connection().await?;
        conn.begin_transaction().await?;
        match f(&mut *conn).await {
            Ok(r) => {
                conn.commit_transaction().await?;
                Ok(r)
            }
            Err(e) => {
                let _ = conn.rollback_transaction().await;
                Err(e)
            }
        }
    }
}

#[async_trait::async_trait]
impl<T: IDbContext + Send + Sync> IDbContextExt for T {}

// ---------------------------------------------------------------------------
// AppDbContext implements IDbContext
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl IDbContext for AppDbContext {
    fn provider(&self) -> &dyn IDatabaseProvider {
        &*self.provider
    }
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker {
        &mut self.change_tracker
    }
    fn change_tracker(&self) -> &ChangeTracker {
        &self.change_tracker
    }

    async fn save_changes(&mut self) -> LrefResult<SaveChangesResult> {
        self.change_tracker.detect_changes();
        let mut conn = self.provider.get_connection().await?;
        conn.begin_transaction().await?;

        let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
        let mut total_added = 0usize;
        let mut total_updated = 0usize;
        let mut total_deleted = 0usize;
        for type_id in &type_ids {
            let saver = self.savers.get(type_id).expect("saver not registered");
            let set = self.sets.get_mut(type_id).unwrap();
            let (a, u, d) = saver
                .save(&mut *conn, &*self.provider, set.as_mut())
                .await?;
            total_added += a;
            total_updated += u;
            total_deleted += d;
        }
        conn.commit_transaction().await?;
        self.change_tracker.accept_all_changes();
        for type_id in &type_ids {
            let saver = self.savers.get(type_id).unwrap();
            let set = self.sets.get_mut(type_id).unwrap();
            saver.clear(set.as_mut());
        }
        Ok(SaveChangesResult {
            added: total_added,
            updated: total_updated,
            deleted: total_deleted,
        })
    }
}

// ---------------------------------------------------------------------------
// save_one_set
// ---------------------------------------------------------------------------

pub async fn save_one_set<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut dyn IDbSet<E>,
) -> LrefResult<(usize, usize, usize)>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow,
{
    let meta = E::entity_meta();
    let added: Vec<(&E, &EntityTypeMeta)> = db_set
        .added_entities()
        .into_iter()
        .map(|e| (e, &meta))
        .collect();
    let modified: Vec<(&E, &EntityTypeMeta)> = db_set
        .modified_entities()
        .into_iter()
        .map(|e| (e, &meta))
        .collect();
    let deleted: Vec<(&E, &EntityTypeMeta)> = db_set
        .deleted_entities()
        .into_iter()
        .map(|e| (e, &meta))
        .collect();
    let mut ac = 0usize;
    let mut uc = 0usize;
    let mut dc = 0usize;
    if !added.is_empty() {
        ac = ChangeExecutor::execute_inserts(conn, provider, &added, |_, _| {}).await?;
    }
    if !modified.is_empty() {
        uc = ChangeExecutor::execute_updates(conn, provider, &modified).await?;
    }
    if !deleted.is_empty() {
        dc = ChangeExecutor::execute_deletes(conn, provider, &deleted).await?;
    }
    Ok((ac, uc, dc))
}

// ---------------------------------------------------------------------------
// save_changes_all! macro
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! save_changes_all {
    ($ctx:expr, $first:expr $(, $rest:expr)* $(,)?) => {{
        $ctx.change_tracker_mut().detect_changes();
        let mut conn = $ctx.provider().get_connection().await?;
        conn.begin_transaction().await?;
        let mut added = 0usize; let mut updated = 0usize; let mut deleted = 0usize;
        { let (a, u, d) = $crate::db_context::save_one_set(&mut *conn, $ctx.provider(), &mut $first).await?; added += a; updated += u; deleted += d; }
        $({ let (a, u, d) = $crate::db_context::save_one_set(&mut *conn, $ctx.provider(), &mut $rest).await?; added += a; updated += u; deleted += d; })*
        conn.commit_transaction().await?;
        $ctx.change_tracker_mut().accept_all_changes();
        $first.clear_entries(); $($rest.clear_entries();)*
        Ok($crate::db_context::SaveChangesResult { added, updated, deleted })
    }};
}

// ---------------------------------------------------------------------------
// SaveChangesResult
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SaveChangesResult {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
}
impl SaveChangesResult {
    pub fn total(&self) -> usize {
        self.added + self.updated + self.deleted
    }
}
impl std::fmt::Display for SaveChangesResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} entities modified ({} added, {} updated, {} deleted)",
            self.total(),
            self.added,
            self.updated,
            self.deleted
        )
    }
}
