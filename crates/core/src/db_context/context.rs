//! `DbContext` struct and non-save methods.
//!
//! `DbContext` is the concrete context type. Entity sets use a type-map:
//! `ctx.set::<Blog>()` lazy-creates `DbSet<Blog>`. `SetOps<T>` dispatchers
//! enable `save_changes()` to iterate all entity types.
//!
//! ## Ownership and Mutation
//!
//! `DbContext` methods (`set::<T>()`, `save_changes()`, `detect_changes()`)
//! require `&mut self` — this is idiomatic Rust, not a limitation. The DI
//! integration (`add_dbcontext`) registers the context as **Scoped** and
//! supports owned resolution via `get_owned::<DbContext>()`.
//!
//! `DbContext` is **not** thread-safe — a single instance must not be shared
//! across threads (aligned with EFCore).

use crate::db_set::DbSet;
use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use crate::error::{EFError, EFResult};
use crate::interceptor::InterceptorPipeline;
use crate::metadata::EntityTypeMeta;
use crate::migration::MigrationEngine;
use crate::model_builder::ModelBuilder;
use crate::provider::{DbValue, IDatabaseProvider};
use crate::tracking::ChangeTracker;
use crate::transaction::{DbTransaction, ITransaction};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::{DbContextOptions, ErasedSetOps, SetOps};

/// The session / unit-of-work entry point.
///
/// Entity sets are lazy-created via `ctx.set::<T>()`. Metadata is loaded from
/// the process-level `MetadataCache` on `DbContextOptions` (built once per
/// `context_key`, then `Arc`-shared). `save_changes()` commits all pending
/// changes in a transaction.
pub struct DbContext {
    pub(crate) sets: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    pub(crate) savers: HashMap<TypeId, Box<dyn ErasedSetOps>>,
    pub(crate) entity_metas: HashMap<TypeId, EntityTypeMeta>,
    pub(crate) model_builder: ModelBuilder,
    pub(crate) change_tracker: ChangeTracker,
    pub(crate) provider: Arc<dyn IDatabaseProvider>,
    pub(crate) interceptor_pipeline: InterceptorPipeline,
    pub(crate) lazy_loading_enabled: bool,
    /// Ambient transaction: registered by `use_transaction()`. When present,
    /// `save_changes()` reuses this transaction's connection and does not
    /// begin/commit/rollback on its own. Uses `take()`/restore pattern to
    /// avoid `&mut self` borrow conflicts with `self.sets` during save.
    pub(crate) ambient_transaction: Option<Box<dyn ITransaction>>,
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

    /// Executes a raw SQL query and materializes the result rows into entities.
    ///
    /// This is the escape hatch for complex queries (multi-table JOINs, CTEs,
    /// window functions) that are hard to express via LINQ. The caller is
    /// responsible for SQL correctness and parameterization.
    ///
    /// # Example
    /// ```rust,ignore
    /// let blogs: Vec<Blog> = ctx
    ///     .sql_query("SELECT * FROM blogs WHERE id = ?", &[DbValue::I32(1)])
    ///     .await?;
    /// ```
    pub async fn sql_query<T: IFromRow + IEntityType>(
        &self,
        sql: &str,
        params: &[DbValue],
    ) -> EFResult<Vec<T>> {
        let mut conn = self.provider.get_connection().await?;
        let rows = conn.query(sql, params).await?;
        crate::entity::materialize_entities(&rows)
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
            .ok_or_else(|| {
                EFError::transaction(
                    "ambient_transaction was consumed during use_transaction closure",
                )
            })?;
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
