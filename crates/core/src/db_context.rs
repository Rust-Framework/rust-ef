//! DbContext trait, DbContextOptions, and ChangeTracker ??the session / unit-of-work layer.
//!
//! ## Architecture
//!
//! `IDbContext` is object-safe ??no `Sized`, no associated type, no generic methods.
//! This enables `dyn IDbContext` resolution from DI containers.
//!
//! Entity sets use a type-map: `ctx.set::<Blog>()` lazy-creates `DbSet<Blog>`.
//! `SetOps<T>` dispatchers enable `save_changes()` to iterate all entity types.
//!
//! ## Provider Factory
//!
//! `DbContextOptions` stores a `provider_factory` closure injected by the
//! provider extension methods (`use_sqlite`, `use_postgres`, `use_mysql`).
//! `DbContext::from_options()` calls this factory to create the provider.

use crate::change_executor::ChangeExecutor;
use crate::db_set::DbSet;
use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use crate::error::{EFError, EFResult};
use crate::interceptor::{InterceptorPipeline, SaveChangesContext, SaveChangesResultContext};
use crate::metadata::EntityTypeMeta;
use crate::migration::MigrationEngine;
use crate::model_builder::ModelBuilder;
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};
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
    #[allow(clippy::type_complexity)]
    pub(crate) provider_factory:
        Option<Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync>>,
    pub(crate) interceptors: Vec<Arc<dyn crate::interceptor::ISaveChangesInterceptor>>,
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
    pub fn create_provider(&self) -> EFResult<Arc<dyn IDatabaseProvider>> {
        let factory = self.provider_factory.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider configured. Call use_sqlite / use_postgres / use_mysql first.".into(),
            )
        })?;
        factory(self.connection_string())
    }
}

#[allow(clippy::derivable_impls)]
impl Default for DbContextOptions {
    fn default() -> Self {
        Self {
            connection_string: String::new(),
            provider_tag: None,
            provider_factory: None,
            interceptors: Vec::new(),
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
    #[allow(clippy::type_complexity)]
    pub fn set_provider_factory(
        &mut self,
        tag: &str,
        cs: impl Into<String>,
        factory: Arc<dyn Fn(&str) -> EFResult<Arc<dyn IDatabaseProvider>> + Send + Sync>,
    ) -> &mut Self {
        self.inner.provider_tag = Some(tag.to_string());
        self.inner.connection_string = cs.into();
        self.inner.provider_factory = Some(factory);
        self
    }
    /// Registers a `SaveChanges` interceptor.
    ///
    /// Interceptors are called in registration order during
    /// `save_changes()`. Use this for auditing, soft-delete,
    /// validation, and other cross-cutting concerns.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// options
    ///     .use_sqlite("app.db")
    ///     .add_interceptor(AuditInterceptor::new());
    /// ```
    pub fn add_interceptor(
        &mut self,
        interceptor: impl crate::interceptor::ISaveChangesInterceptor + 'static,
    ) -> &mut Self {
        self.inner.interceptors.push(Arc::new(interceptor));
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
    ) -> EFResult<(usize, usize, usize)>;
    fn detect_changes(&self, raw_set: &mut (dyn Any + Send + Sync));
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
    E: IEntityType
        + IEntitySnapshot
        + IGetKeyValues
        + IFromRow
        + INavigationSetter
        + Send
        + Sync
        + 'static,
{
    async fn save(
        &self,
        conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
    ) -> EFResult<(usize, usize, usize)> {
        let db_set = raw_set
            .downcast_mut::<DbSet<E>>()
            .expect("SetOps type mismatch");
        save_one_set(conn, provider, db_set).await
    }
    fn detect_changes(&self, raw_set: &mut (dyn Any + Send + Sync)) {
        if let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() {
            db_set.detect_changes();
        }
    }
    fn clear(&self, raw_set: &mut (dyn Any + Send + Sync + 'static)) {
        if let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() {
            db_set.clear_entries();
        }
    }
}

// ---------------------------------------------------------------------------
// DbContext
// ---------------------------------------------------------------------------

pub struct DbContext {
    sets: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    savers: HashMap<TypeId, Box<dyn ErasedSetOps>>,
    entity_metas: HashMap<TypeId, EntityTypeMeta>,
    model_builder: ModelBuilder,
    change_tracker: ChangeTracker,
    provider: Arc<dyn IDatabaseProvider>,
    interceptor_pipeline: InterceptorPipeline,
}

impl DbContext {
    /// Creates the context from options (uses the provider factory stored in options).
    pub fn from_options(options: &DbContextOptions) -> EFResult<Self> {
        let provider = options.create_provider()?;
        Ok(Self {
            sets: HashMap::new(),
            savers: HashMap::new(),
            entity_metas: HashMap::new(),
            model_builder: ModelBuilder::new(),
            change_tracker: ChangeTracker::new(),
            provider,
            interceptor_pipeline: InterceptorPipeline::new(options.interceptors.clone()),
        })
    }

    pub fn set<T>(&mut self) -> &mut DbSet<T>
    where
        T: IEntityType
            + IEntitySnapshot
            + IGetKeyValues
            + IFromRow
            + INavigationSetter
            + Send
            + Sync
            + 'static,
    {
        let type_id = TypeId::of::<T>();
        self.savers
            .entry(type_id)
            .or_insert_with(|| Box::new(SetOps::<T>::new()));
        self.entity_metas
            .entry(type_id)
            .or_insert_with(T::entity_meta);
        self.sets.entry(type_id).or_insert_with(|| {
            let meta = T::entity_meta();
            let mut db_set =
                DbSet::<T>::with_provider(meta.table_name.as_ref(), Arc::clone(&self.provider));
            if let Some(filter) = self.model_builder.get_query_filter(&type_id) {
                db_set.set_query_filter(filter.clone());
            }
            Box::new(db_set)
        });
        self.sets
            .get_mut(&type_id)
            .and_then(|b| b.downcast_mut::<DbSet<T>>())
            .expect("DbSet type mismatch")
    }

    /// Returns the model builder for Fluent API configuration.
    pub fn model(&mut self) -> &mut ModelBuilder {
        &mut self.model_builder
    }

    /// Detects changes on all tracked DbSets by comparing property snapshots.
    pub fn detect_changes(&mut self) {
        let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
        for type_id in type_ids {
            if let Some(set) = self.sets.get_mut(&type_id) {
                if let Some(saver) = self.savers.get(&type_id) {
                    saver.detect_changes(set.as_mut());
                }
            }
        }
    }

    /// Creates all tables for entity types registered via `set::<T>()`.
    /// Corresponds to EF Core `Database.EnsureCreated()`.
    pub async fn ensure_created(&self) -> EFResult<()> {
        let metas: Vec<EntityTypeMeta> = self.entity_metas.values().cloned().collect();
        if metas.is_empty() {
            return Err(EFError::Configuration(
                "No entity types registered. Call ctx.set::<T>() before ensure_created().".into(),
            ));
        }
        let dialect = self.provider.migration_dialect();
        MigrationEngine::new(dialect)
            .ensure_created(&*self.provider, &metas)
            .await?;

        for (type_id, meta) in &self.entity_metas {
            let rows = self.model_builder.seed_rows_for(type_id);
            if !rows.is_empty() {
                MigrationEngine::new(dialect)
                    .apply_seed_data(&*self.provider, meta, rows)
                    .await?;
            }
        }
        Ok(())
    }

    /// Drops all tables for entity types registered via `set::<T>()`.
    /// Corresponds to EF Core `Database.EnsureDeleted()`.
    pub async fn ensure_deleted(&self) -> EFResult<()> {
        let metas: Vec<EntityTypeMeta> = self.entity_metas.values().cloned().collect();
        if metas.is_empty() {
            return Err(EFError::Configuration(
                "No entity types registered. Call ctx.set::<T>() before ensure_deleted().".into(),
            ));
        }
        let dialect = self.provider.migration_dialect();
        MigrationEngine::new(dialect)
            .ensure_deleted(&*self.provider, &metas)
            .await
    }
}

// ---------------------------------------------------------------------------
// IDbContext ??object-safe
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
pub trait IDbContext: Send + Sync {
    fn provider(&self) -> &dyn IDatabaseProvider;
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker;
    fn change_tracker(&self) -> &ChangeTracker;
    async fn save_changes(&mut self) -> EFResult<SaveChangesResult>;

    async fn begin_transaction(&self) -> EFResult<Box<dyn IAsyncConnection>> {
        let mut conn = self.provider().get_connection().await?;
        conn.begin_transaction().await?;
        Ok(conn)
    }
}

#[async_trait::async_trait]
pub trait IDbContextExt: IDbContext {
    async fn use_transaction<F, Fut, R>(&self, f: F) -> EFResult<R>
    where
        F: FnOnce(&mut dyn IAsyncConnection) -> Fut + Send,
        Fut: Future<Output = EFResult<R>> + Send,
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
// DbContext implements IDbContext
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl IDbContext for DbContext {
    fn provider(&self) -> &dyn IDatabaseProvider {
        &*self.provider
    }
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker {
        &mut self.change_tracker
    }
    fn change_tracker(&self) -> &ChangeTracker {
        &self.change_tracker
    }

    async fn save_changes(&mut self) -> EFResult<SaveChangesResult> {
        let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
        for type_id in &type_ids {
            let set = self.sets.get_mut(type_id).unwrap();
            self.savers
                .get(type_id)
                .unwrap()
                .detect_changes(set.as_mut());
        }

        // --- Interceptor: on_saving (pre-commit) ---
        let save_ctx = SaveChangesContext::from_tracker(&self.change_tracker);
        self.interceptor_pipeline.on_saving(&save_ctx).await?;

        let mut conn = self.provider.get_connection().await?;
        conn.begin_transaction().await?;

        let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
        let mut total_added = 0usize;
        let mut total_updated = 0usize;
        let mut total_deleted = 0usize;
        for type_id in &type_ids {
            let saver = self.savers.get(type_id).expect("saver not registered");
            let set = self.sets.get_mut(type_id).unwrap();
            let (a, u, d) = match saver.save(&mut *conn, &*self.provider, set.as_mut()).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = conn.rollback_transaction().await;
                    self.interceptor_pipeline
                        .on_save_failed(&save_ctx, &e)
                        .await;
                    return Err(e);
                }
            };
            total_added += a;
            total_updated += u;
            total_deleted += d;
        }
        if let Err(e) = conn.commit_transaction().await {
            self.interceptor_pipeline
                .on_save_failed(&save_ctx, &e)
                .await;
            return Err(e);
        }
        self.change_tracker.accept_all_changes();
        for type_id in &type_ids {
            let saver = self.savers.get(type_id).unwrap();
            let set = self.sets.get_mut(type_id).unwrap();
            saver.clear(set.as_mut());
        }

        // --- Interceptor: on_saved (post-commit) ---
        let result_ctx = SaveChangesResultContext {
            added: total_added,
            updated: total_updated,
            deleted: total_deleted,
        };
        self.interceptor_pipeline
            .on_saved(&save_ctx, &result_ctx)
            .await?;

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

#[allow(clippy::type_complexity)]
pub async fn save_one_set<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
) -> EFResult<(usize, usize, usize)>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow,
{
    let meta = E::entity_meta();
    let added: Vec<(&E, &EntityTypeMeta)> = db_set
        .tracked_by_state(crate::entity::EntityState::Added)
        .into_iter()
        .map(|(e, _)| (e, &meta))
        .collect();
    let modified: Vec<(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)> = db_set
        .tracked_by_state(crate::entity::EntityState::Modified)
        .into_iter()
        .map(|(e, orig)| (e, &meta, orig))
        .collect();
    let deleted: Vec<(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)> = db_set
        .tracked_by_state(crate::entity::EntityState::Deleted)
        .into_iter()
        .map(|(e, orig)| (e, &meta, orig))
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
