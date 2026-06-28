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
//!
//! ## Thread Safety
//!
//! `DbContext` is **not** thread-safe — a single instance must not be shared
//! across threads. This is a design decision (aligned with EFCore), not a
//! limitation.
//!
//! **Correct usage**: create one DI `Scope` per request / operation:
//! ```rust,ignore
//! let scope = provider.create_scope();
//! let ctx = scope.get::<dyn IDbContext>().unwrap();
//! // Multiple `get` calls within the same scope return the same instance
//! // (unit-of-work semantics).
//! ```
//!
//! > **rust-webapp**: the HTTP pipeline manages scopes automatically.
//! > Handlers simply declare `ctx: Arc<dyn IDbContext>` — no manual
//! > `create_scope()` needed.
//!
//! **Anti-pattern**: sharing via `Arc<Mutex<DbContext>>` causes tracking
//! pollution — Thread A's `save_changes()` would commit Thread B's pending
//! changes.
//!
//! Resolving `dyn IDbContext` directly from the root `ServiceProvider`
//! degrades to a fresh instance per call (equivalent to transient).

use crate::change_executor::ChangeExecutor;
use crate::db_set::DbSet;
use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use crate::error::{EFError, EFResult};
use crate::interceptor::{InterceptorPipeline, SaveChangesContext, SaveChangesResultContext};
use crate::metadata::EntityTypeMeta;
use crate::migration::MigrationEngine;
use crate::model_builder::ModelBuilder;
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};
use crate::registration::{EntityConfigRegistration, EntityRegistration};
use crate::tracking::{ChangeTracker, EntityEntryView};
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
    /// When `true`, `QueryBuilder::to_list()` attaches `LazyContext` to every
    /// navigation container on materialized entities, enabling on-demand
    /// loading via `BelongsTo::load()` / `HasMany::load()` / `HasOne::load()`.
    ///
    /// Defaults to `false` (opt-in) to preserve v1.0 eager-only behavior.
    pub(crate) lazy_loading_enabled: bool,
    pub(crate) context_key: Option<String>,
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
    pub fn lazy_loading_enabled(&self) -> bool {
        self.lazy_loading_enabled
    }
    pub fn context_key(&self) -> Option<&str> {
        self.context_key.as_deref()
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
            lazy_loading_enabled: false,
            context_key: None,
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

    /// Enables or disables lazy loading of navigation properties.
    ///
    /// When enabled (`true`), `to_list()` attaches a `LazyContext` to every
    /// navigation container on each materialized entity. The user can then
    /// call `nav.load().await` to trigger a single-entity query on first
    /// access; subsequent accesses read from the in-memory cache.
    ///
    /// When disabled (`false`, the default), navigation properties are
    /// empty unless explicitly loaded via `Include` — matching v1.0
    /// eager-only behavior.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut options = DbContextOptionsBuilder::new();
    /// options.use_sqlite_in_memory().use_lazy_loading(true);
    /// ```
    pub fn use_lazy_loading(&mut self, enabled: bool) -> &mut Self {
        self.inner.lazy_loading_enabled = enabled;
        self
    }

    /// Sets the context key used to filter entities and configurations
    /// during `DbContext::discover_entities()`. Set automatically by
    /// `add_dbcontext_keyed`; `None` (the default) selects the default
    /// context.
    pub fn context_key(&mut self, key: impl Into<String>) -> &mut Self {
        self.inner.context_key = Some(key.into());
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
        meta: &EntityTypeMeta,
    ) -> EFResult<(usize, usize, usize)>;
    fn detect_changes(&self, raw_set: &mut (dyn Any + Send + Sync));
    fn clear(&self, raw_set: &mut (dyn Any + Send + Sync + 'static));
    /// Collects type-erased views of all pending entries in the set, used to
    /// build `SaveChangesContext` from the real save data source (`DbSet.entries`)
    /// rather than the legacy (empty) `change_tracker`.
    fn collect_entries(&self, raw_set: &(dyn Any + Send + Sync)) -> Vec<EntityEntryView>;
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
        meta: &EntityTypeMeta,
    ) -> EFResult<(usize, usize, usize)> {
        let db_set = raw_set
            .downcast_mut::<DbSet<E>>()
            .expect("SetOps type mismatch");
        save_one_set(conn, provider, db_set, meta).await
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
    fn collect_entries(&self, raw_set: &(dyn Any + Send + Sync)) -> Vec<EntityEntryView> {
        let Some(db_set) = raw_set.downcast_ref::<DbSet<E>>() else {
            return Vec::new();
        };
        let type_name = E::entity_meta().type_name.to_string();
        db_set
            .entries
            .iter()
            .map(|e| EntityEntryView {
                type_id: TypeId::of::<E>(),
                type_name: type_name.clone(),
                state: e.state,
            })
            .collect()
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
    lazy_loading_enabled: bool,
    context_key: Option<String>,
}

impl DbContext {
    /// Creates the context from options (uses the provider factory stored in options).
    pub fn from_options(options: &DbContextOptions) -> EFResult<Self> {
        let provider = options.create_provider()?;
        let mut ctx = Self {
            sets: HashMap::new(),
            savers: HashMap::new(),
            entity_metas: HashMap::new(),
            model_builder: ModelBuilder::new(),
            change_tracker: ChangeTracker::new(),
            provider,
            interceptor_pipeline: InterceptorPipeline::new(options.interceptors.clone()),
            lazy_loading_enabled: options.lazy_loading_enabled,
            context_key: options.context_key.clone(),
        };
        // Auto-discover all entities registered via #[derive(EntityType)] and
        // apply all #[entity(T)] configurations. This is idempotent — manual
        // discover_entities() calls after from_options() are safe no-ops.
        ctx.discover_entities()?;
        Ok(ctx)
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
        if !self.model_builder.has_entity(type_id) {
            self.model_builder.register_entity_meta(T::entity_meta());
        }
        self.sets.entry(type_id).or_insert_with(|| {
            let table_name = self
                .model_builder
                .build()
                .into_iter()
                .find(|m| m.type_id == type_id)
                .map(|m| m.table_name.to_string())
                .unwrap_or_else(|| T::entity_meta().table_name.to_string());
            let mut db_set = DbSet::<T>::with_provider(table_name, Arc::clone(&self.provider));
            if let Some(filter) = self.model_builder.get_query_filter(&type_id) {
                db_set.set_query_filter(filter.clone());
            }
            db_set.set_filter_map(self.model_builder.filters_by_table());
            db_set.set_lazy_loading_enabled(self.lazy_loading_enabled);
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

    /// Returns a read-only reference to the model builder.
    pub fn model_builder(&self) -> &ModelBuilder {
        &self.model_builder
    }

    /// Returns `true` if an entity of type `T` has been discovered and
    /// registered in the entity metadata map.
    pub fn entity_metas_contains<T: IEntityType>(&self) -> bool {
        self.entity_metas.contains_key(&TypeId::of::<T>())
    }

    /// Discovers all entity types registered via `#[derive(EntityType)]`
    /// and applies all `#[entity(T)]` configurations to the model builder.
    ///
    /// After calling this, `ensure_created()` and `ensure_deleted()` will
    /// process all discovered entities without requiring manual `set::<T>()`
    /// calls. Calling `set::<T>()` for discovered entities is idempotent —
    /// it only creates the `DbSet` instance and `SetOps` saver, since the
    /// metadata is already present.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut ctx = DbContext::from_options(&options)?;
    /// // discover_entities() is called automatically by from_options()
    /// ctx.ensure_created().await?;
    /// ```
    pub fn discover_entities(&mut self) -> EFResult<()> {
        let my_key = self.context_key.as_deref();

        // Apply Fluent configurations matching this context's key.
        for reg in inventory::iter::<EntityConfigRegistration> {
            if reg.context_key == my_key {
                (reg.apply_fn)(&mut self.model_builder);
            }
        }

        // Register entity metadata for entities matching this context's key.
        for reg in inventory::iter::<EntityRegistration> {
            if reg.context_key == my_key {
                let meta = reg.meta();
                let type_id = reg.type_id;
                self.entity_metas
                    .entry(type_id)
                    .or_insert_with(|| meta.clone());
                if !self.model_builder.has_entity(type_id) {
                    self.model_builder.register_entity_meta(meta);
                }
            }
        }

        Ok(())
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

    /// Builds the interceptor `SaveChangesContext` from the actual pending
    /// entries across all `DbSet`s (the real save data source), instead of
    /// the legacy `change_tracker` which is never populated by `DbSet::add`.
    /// This keeps interceptor snapshots consistent with what will be committed.
    fn build_save_context(&self) -> SaveChangesContext {
        let mut views: Vec<EntityEntryView> = Vec::new();
        for (type_id, set) in &self.sets {
            if let Some(saver) = self.savers.get(type_id) {
                views.extend(saver.collect_entries(set.as_ref()));
            }
        }
        SaveChangesContext::from_views(views)
    }

    /// Creates all tables for registered entity types.
    ///
    /// Sources metas from `model_builder.build()`, which applies all Fluent
    /// API configurations and `#[entity(T)]` overrides. Entities are
    /// discovered automatically via `#[derive(EntityType)]`; call
    /// `discover_entities()` first, or use `set::<T>()` to register manually.
    pub async fn ensure_created(&self) -> EFResult<()> {
        let mut metas: Vec<EntityTypeMeta> = self.model_builder.build();
        if metas.is_empty() {
            metas = self.entity_metas.values().cloned().collect();
        }
        if metas.is_empty() {
            return Err(EFError::Configuration(
                "No entity types registered. Call ctx.discover_entities() or ctx.set::<T>() before ensure_created().".into(),
            ));
        }
        let dialect = self.provider.migration_dialect();
        MigrationEngine::new(dialect)
            .ensure_created(&*self.provider, &metas)
            .await?;

        for meta in &metas {
            let rows = self.model_builder.seed_rows_for(&meta.type_id);
            if !rows.is_empty() {
                MigrationEngine::new(dialect)
                    .apply_seed_data(&*self.provider, meta, rows)
                    .await?;
            }
        }
        Ok(())
    }

    /// Drops all tables for registered entity types.
    pub async fn ensure_deleted(&self) -> EFResult<()> {
        let mut metas: Vec<EntityTypeMeta> = self.model_builder.build();
        if metas.is_empty() {
            metas = self.entity_metas.values().cloned().collect();
        }
        if metas.is_empty() {
            return Err(EFError::Configuration(
                "No entity types registered. Call ctx.discover_entities() or ctx.set::<T>() before ensure_deleted().".into(),
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

        // Build configured metas from model_builder so that Fluent API overrides
        // (to_table, has_column_name, etc.) are respected during save operations.
        let configured_metas: HashMap<TypeId, EntityTypeMeta> = self
            .model_builder
            .build()
            .into_iter()
            .map(|m| (m.type_id, m))
            .collect();

        // --- Interceptor: on_saving (pre-commit) ---
        // Build the context from the actual pending entries across all DbSets
        // (the real save data source), not the legacy (empty) change_tracker.
        let save_ctx = self.build_save_context();
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
            let meta = configured_metas
                .get(type_id)
                .or_else(|| self.entity_metas.get(type_id))
                .expect("meta not found for entity type");
            let (a, u, d) = match saver
                .save(&mut *conn, &*self.provider, set.as_mut(), meta)
                .await
            {
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
    meta: &EntityTypeMeta,
) -> EFResult<(usize, usize, usize)>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow,
{
    let query_filter = db_set.query_filter();

    let added: Vec<(&E, &EntityTypeMeta)> = db_set
        .tracked_by_state(crate::entity::EntityState::Added)
        .into_iter()
        .map(|(e, _)| (e, meta))
        .collect();
    let modified: Vec<(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)> = db_set
        .tracked_by_state(crate::entity::EntityState::Modified)
        .into_iter()
        .map(|(e, orig)| (e, meta, orig))
        .collect();
    let deleted: Vec<(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)> = db_set
        .tracked_by_state(crate::entity::EntityState::Deleted)
        .into_iter()
        .map(|(e, orig)| (e, meta, orig))
        .collect();
    let mut ac = 0usize;
    let mut uc = 0usize;
    let mut dc = 0usize;
    if !added.is_empty() {
        ac = ChangeExecutor::execute_inserts(conn, provider, &added, |_, _| {}).await?;
    }
    if !modified.is_empty() {
        uc = ChangeExecutor::execute_updates(conn, provider, &modified, query_filter).await?;
    }
    if !deleted.is_empty() {
        dc = ChangeExecutor::execute_deletes(conn, provider, &deleted, query_filter).await?;
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
