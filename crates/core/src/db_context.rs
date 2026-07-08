//! DbContext, DbContextOptions, and ChangeTracker — the session / unit-of-work layer.
//!
//! ## Architecture
//!
//! `DbContext` is the concrete context type. Entity sets use a type-map:
//! `ctx.set::<Blog>()` lazy-creates `DbSet<Blog>`. `SetOps<T>` dispatchers
//! enable `save_changes()` to iterate all entity types.
//!
//! ## Provider Factory
//!
//! `DbContextOptions` stores a `provider_factory` closure injected by the
//! provider extension methods (`use_sqlite`, `use_postgres`, `use_mysql`).
//! `DbContext::from_options()` calls this factory to create the provider.
//!
//! ## Ownership and Mutation
//!
//! `DbContext` methods (`set::<T>()`, `save_changes()`, `detect_changes()`)
//! require `&mut self` — this is idiomatic Rust, not a limitation. The DI
//! integration (`add_dbcontext`) registers the context as **Scoped** and
//! supports two resolution modes:
//!
//! - **Owned** (recommended for handlers): `provider.get_owned::<DbContext>()`
//!   returns a fresh `DbContext` with direct `&mut self` access. Handlers
//!   declare a bare `ctx: DbContext` field marked with `#[inject(owned)]`;
//!   `#[derive(Inject)]` resolves it via `get_owned()`. Unmarked fields fall
//!   back to `Default::default()`.
//! - **Shared** (within a scope): `scope.get::<DbContext>()` returns
//!   `Arc<DbContext>` for consumers that only need `&self` access.
//!
//! ```rust,ignore
//! // Owned — idiomatic &mut self, no locks:
//! // rust-dix 0.6+: get_owned() returns Result<T, RdiError>
//! let mut ctx: DbContext = provider.get_owned()?;
//! ctx.set::<Blog>().add(blog);
//! ctx.save_changes().await?;
//! ```
//!
//! ## Thread Safety
//!
//! `DbContext` is **not** thread-safe — a single instance must not be shared
//! across threads. This is a design decision (aligned with EFCore), not a
//! limitation.
//!
//! **Correct usage**: each request / operation owns its own `DbContext`
//! instance (via `get_owned()` or a fresh scope):
//! ```rust,ignore
//! let mut ctx: DbContext = provider.get_owned()?;
//! // This instance is exclusively owned — &mut self works directly.
//! ```
//!
//! > **rust-webapp**: the HTTP pipeline creates a DI scope per request and
//! > resolves handlers via `get_owned::<Handler>()`. Handlers own a fresh
//! > `DbContext` — no manual scope management needed.
//!
//! **Anti-pattern**: sharing via `Arc<Mutex<DbContext>>` causes tracking
//! pollution — Thread A's `save_changes()` would commit Thread B's pending
//! changes. Prefer owned resolution.

use crate::change_executor::ChangeExecutor;
use crate::db_set::DbSet;
use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use crate::error::{EFError, EFResult};
use crate::interceptor::{InterceptorPipeline, SaveChangesContext, SaveChangesResultContext};
use crate::metadata::EntityTypeMeta;
use crate::migration::MigrationEngine;
use crate::model_builder::ModelBuilder;
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};
use crate::tracking::{ChangeTracker, EntityEntryView};
use crate::transaction::{DbTransaction, ITransaction};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
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
    /// Process-level cache of the built provider (which owns the connection
    /// pool). Built once on the first `create_provider()` call and shared
    /// across every `DbContext` created from the same `Arc<DbContextOptions>`
    /// (i.e. the same `add_dbcontext` registration). Keeping the provider
    /// alive for the application lifetime means the connection pool is reused
    /// across requests instead of being recreated per request.
    pub(crate) provider_cache: Arc<std::sync::Mutex<Option<Arc<dyn IDatabaseProvider>>>>,
    pub(crate) interceptors: Vec<Arc<dyn crate::interceptor::ISaveChangesInterceptor>>,
    /// When `true`, `QueryBuilder::to_list()` attaches `LazyContext` to every
    /// navigation container on materialized entities, enabling on-demand
    /// loading via `BelongsTo::load()` / `HasMany::load()` / `HasOne::load()`.
    ///
    /// Defaults to `false` (opt-in) to preserve v1.0 eager-only behavior.
    pub(crate) lazy_loading_enabled: bool,
    pub(crate) context_key: Option<String>,
    /// Process-level cache of `discover_entities()` output, keyed by
    /// `context_key`. Shared across all `DbContext` instances created from
    /// the same `DbContextOptions` (which is `Arc`-shared per `add_dbcontext`
    /// registration). The first `from_options()` call builds the metadata;
    /// subsequent calls `Arc::clone` it.
    pub(crate) metadata_cache: Arc<crate::metadata_cache::MetadataCache>,
    /// Slow query threshold for tracing. When set, queries exceeding this
    /// duration emit a `tracing::warn!` event.
    #[cfg(feature = "tracing")]
    pub(crate) slow_query_threshold: Option<std::time::Duration>,
}

impl std::fmt::Debug for DbContextOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbContextOptions")
            .field(
                "connection_string",
                &redact_connection_string(&self.connection_string),
            )
            .field("provider_tag", &self.provider_tag)
            .finish()
    }
}

/// Redacts credentials from a connection string so `Debug` output never leaks
/// passwords. Handles URL form (`scheme://user:pass@host`) and key=value form
/// (`...;Password=...;...`). SQLite file paths and other credential-free
/// strings are returned unchanged for debuggability.
fn redact_connection_string(cs: &str) -> String {
    // URL form: scheme://[user[:pass]@]host...
    if let Some(scheme_end) = cs.find("://") {
        let (scheme, rest) = cs.split_at(scheme_end + 3);
        if let Some(at) = rest.find('@') {
            let (userinfo, host_and_rest) = rest.split_at(at);
            let redacted_user = match userinfo.find(':') {
                Some(colon) => &userinfo[..colon],
                None => userinfo,
            };
            return format!("{}{}***@{}", scheme, redacted_user, &host_and_rest[1..]);
        }
        return cs.to_string();
    }
    // Key=value form: redact any token whose key mentions password/pwd.
    if cs.contains('=') {
        return cs
            .split(';')
            .map(|pair| {
                let eq = match pair.find('=') {
                    Some(e) => e,
                    None => return pair.to_string(),
                };
                let key = pair[..eq].trim().to_lowercase();
                if key.contains("password") || key.contains("pwd") {
                    format!("{}=***", &pair[..eq])
                } else {
                    pair.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(";");
    }
    cs.to_string()
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
        // Recover from a poisoned lock rather than panicking (consistent with
        // `MetadataCache`): if a previous build panicked, `into_inner()` yields
        // the still-`None` cache and we retry the build below.
        let mut guard = self
            .provider_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(provider) = guard.as_ref() {
            return Ok(Arc::clone(provider));
        }
        let factory = self.provider_factory.as_ref().ok_or_else(|| {
            crate::error::EFError::configuration(
                "No provider configured. Call use_sqlite / use_postgres / use_mysql first.",
            )
        })?;
        let provider = factory(self.connection_string())?;
        #[cfg(feature = "tracing")]
        if let Some(threshold) = self.slow_query_threshold {
            provider.set_slow_query_threshold(threshold);
        }
        *guard = Some(Arc::clone(&provider));
        Ok(provider)
    }
}

#[allow(clippy::derivable_impls)]
impl Default for DbContextOptions {
    fn default() -> Self {
        Self {
            connection_string: String::new(),
            provider_tag: None,
            provider_factory: None,
            provider_cache: Arc::new(std::sync::Mutex::new(None)),
            interceptors: Vec::new(),
            lazy_loading_enabled: false,
            context_key: None,
            metadata_cache: Arc::new(crate::metadata_cache::MetadataCache::new()),
            #[cfg(feature = "tracing")]
            slow_query_threshold: None,
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

    /// Sets the slow query threshold. Queries exceeding this duration
    /// emit a `tracing::warn!` event with SQL and elapsed time.
    ///
    /// Only available when the `tracing` feature is enabled.
    #[cfg(feature = "tracing")]
    pub fn slow_query_threshold(&mut self, threshold: std::time::Duration) -> &mut Self {
        self.inner.slow_query_threshold = Some(threshold);
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
    /// Ambient transaction: registered by `use_transaction()`. When present,
    /// `save_changes()` reuses this transaction's connection and does not
    /// begin/commit/rollback on its own. Uses `take()`/restore pattern to
    /// avoid `&mut self` borrow conflicts with `self.sets` during save.
    ///
    /// Note: `begin_transaction()` returns a handle without registering it
    /// here — only `use_transaction()` registers an ambient. This separates
    /// manual handle-based control from scoped ambient control.
    ambient_transaction: Option<Box<dyn ITransaction>>,
}

impl DbContext {
    /// Creates the context from options (uses the provider factory stored in options).
    ///
    /// Entity metadata is loaded from the process-level `MetadataCache` on
    /// `DbContextOptions` — `inventory::iter` + `IEntityTypeConfiguration::configure()`
    /// run once per `context_key` (first call), then the result is `Arc`-shared
    /// across all `DbContext` instances. Per-instance `ModelBuilder` mutations
    /// (`has_query_filter`, etc.) after construction only affect this instance.
    pub fn from_options(options: &DbContextOptions) -> EFResult<Self> {
        let provider = options.create_provider()?;
        let built = options
            .metadata_cache
            .get_or_build(options.context_key.as_deref());
        let ctx = Self {
            sets: HashMap::new(),
            savers: HashMap::new(),
            entity_metas: built.entity_metas.clone(),
            model_builder: ModelBuilder::from_built(&built),
            change_tracker: ChangeTracker::new(),
            provider,
            interceptor_pipeline: InterceptorPipeline::new(options.interceptors.clone()),
            lazy_loading_enabled: options.lazy_loading_enabled,
            ambient_transaction: None,
        };
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

    /// No-op since metadata caching was introduced.
    ///
    /// Historically, this method iterated `inventory::iter` to register entity
    /// metadata and apply `#[entity(T)]` configurations. That work now happens
    /// in `MetadataCache::build()`, invoked lazily by `from_options()` via
    /// `MetadataCache::get_or_build()`. The result is `Arc`-shared across all
    /// `DbContext` instances with the same `context_key`.
    ///
    /// Retained as a public method for backward compatibility — existing code
    /// calling `ctx.discover_entities()?` after `from_options()` continues to
    /// compile and run (as a no-op). The metadata is already populated.
    pub fn discover_entities(&mut self) -> EFResult<()> {
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
            return Err(EFError::configuration(
                "No entity types registered. Call ctx.discover_entities() or ctx.set::<T>() before ensure_created().",
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
            return Err(EFError::configuration(
                "No entity types registered. Call ctx.discover_entities() or ctx.set::<T>() before ensure_deleted().",
            ));
        }
        let dialect = self.provider.migration_dialect();
        MigrationEngine::new(dialect)
            .ensure_deleted(&*self.provider, &metas)
            .await
    }
}

// ---------------------------------------------------------------------------
// DbContext inherent methods (formerly on IDbContext / IDbContextExt traits)
// ---------------------------------------------------------------------------

impl DbContext {
    /// Returns the database provider.
    pub fn provider(&self) -> &dyn IDatabaseProvider {
        &*self.provider
    }

    /// Returns a read-only reference to the change tracker.
    pub fn change_tracker(&self) -> &ChangeTracker {
        &self.change_tracker
    }

    /// Returns a mutable reference to the change tracker.
    pub fn change_tracker_mut(&mut self) -> &mut ChangeTracker {
        &mut self.change_tracker
    }

    /// Returns a mutable reference to the ambient transaction handle, if one
    /// is active (registered by `use_transaction()`).
    ///
    /// Inside a `use_transaction()` closure, use this to access the ambient
    /// handle for savepoint and isolation operations. The borrow must be
    /// released (by scoping) before calling `save_changes()` or other `&mut
    /// self` methods.
    pub fn transaction_mut(&mut self) -> Option<&mut Box<dyn ITransaction>> {
        self.ambient_transaction.as_mut()
    }

    /// Begins a transaction and returns a typed handle.
    ///
    /// The returned `ITransaction` handle is **not** registered as ambient —
    /// `save_changes()` calls will continue to self-manage their own
    /// transactions. Use this when you need explicit control via
    /// `txn.commit()` / `txn.rollback()` / `txn.create_point()` etc.
    ///
    /// For scoped ambient transactions where `save_changes()` should reuse
    /// the same transaction, use [`DbContext::use_transaction`] instead.
    ///
    /// `commit` / `rollback` consume the handle by value, preventing
    /// use-after-commit at the type level.
    pub async fn begin_transaction(&mut self) -> EFResult<Box<dyn ITransaction>> {
        let mut conn = self.provider.get_connection().await?;
        conn.begin_transaction().await?;
        Ok(Box::new(DbTransaction::new(conn)))
    }

    /// Saves all pending changes across all DbSets.
    ///
    /// Detects changes, runs interceptors, executes INSERT/UPDATE/DELETE in a
    /// transaction, and clears tracked entries on success.
    pub async fn save_changes(&mut self) -> EFResult<SaveChangesResult> {
        let _save_guard = crate::observability::SaveChangesGuard::new();
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

        // === Transaction connection acquisition ===
        // If an ambient transaction exists (registered by `use_transaction`),
        // take it out and reuse its connection (no begin/commit/rollback —
        // the outer scope manages that). Otherwise, self-manage a fresh
        // transaction (original behavior).
        enum TxnSource {
            Ambient(Box<dyn ITransaction>),
            Managed(Box<dyn IAsyncConnection>),
        }
        let mut txn = match self.ambient_transaction.take() {
            Some(t) => TxnSource::Ambient(t),
            None => {
                let mut c = self.provider.get_connection().await?;
                c.begin_transaction().await?;
                TxnSource::Managed(c)
            }
        };

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
            let conn_ref: &mut dyn IAsyncConnection = match &mut txn {
                TxnSource::Ambient(t) => t.connection(),
                TxnSource::Managed(c) => c.as_mut(),
            };
            let (a, u, d) = match saver
                .save(conn_ref, &*self.provider, set.as_mut(), meta)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if let TxnSource::Managed(mut conn) = txn {
                        let _ = conn.rollback_transaction().await;
                    } else if let TxnSource::Ambient(t) = txn {
                        self.ambient_transaction = Some(t);
                    }
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
        match txn {
            TxnSource::Ambient(t) => {
                self.ambient_transaction = Some(t);
            }
            TxnSource::Managed(mut conn) => {
                if let Err(e) = conn.commit_transaction().await {
                    self.interceptor_pipeline
                        .on_save_failed(&save_ctx, &e)
                        .await;
                    return Err(e);
                }
            }
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

    /// Executes a closure within an ambient transaction.
    ///
    /// Registers the transaction as ambient for the duration of `f`, so that
    /// `save_changes()` calls inside `f` reuse the same transaction. Commits
    /// on `Ok`, rolls back on `Err`.
    ///
    /// The closure receives `&mut DbContext` and must return a pinned boxed
    /// future. This signature works around Rust's async borrow checker by
    /// letting the closure capture `ctx` by mutable reference while still
    /// producing a `Send` future.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// ctx.use_transaction(|ctx| Box::pin(async move {
    ///     ctx.set::<Blog>().add(blog);
    ///     ctx.save_changes().await?;
    ///     Ok(())
    /// })).await?;
    /// ```
    pub async fn use_transaction<F, R>(&mut self, f: F) -> EFResult<R>
    where
        for<'a> F: FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = EFResult<R>> + Send + 'a>>,
        R: Send + 'static,
    {
        if self.ambient_transaction.is_some() {
            return Err(EFError::transaction(
                "ambient transaction already active; nested use_transaction is not supported",
            ));
        }
        let mut conn = self.provider.get_connection().await?;
        conn.begin_transaction().await?;
        self.ambient_transaction = Some(Box::new(DbTransaction::new(conn)));
        let result = f(self).await;
        let txn = self
            .ambient_transaction
            .take()
            .expect("ambient_transaction set above");
        match result {
            Ok(r) => {
                txn.commit().await?;
                Ok(r)
            }
            Err(e) => {
                let _ = txn.rollback().await;
                Err(e)
            }
        }
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
