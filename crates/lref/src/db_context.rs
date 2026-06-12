//! DbContext trait, DbContextOptions, and ChangeTracker — the session / unit-of-work layer.
//!
//! Provides `save_changes_all!()` macro for batch entity saving and
//! the standalone `save_one_set()` function for single-type saves.

use crate::change_executor::ChangeExecutor;
use crate::db_set::IDbSet;
use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues};
use crate::error::LrefResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{IAsyncConnection, IDatabaseProvider};
use crate::tracking::ChangeTracker;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// DbContextOptions / DbContextOptionsBuilder
// ---------------------------------------------------------------------------

/// Configuration for a DbContext, built via `DbContextOptionsBuilder`.
///
/// Provider crates populate this through extension methods like `use_sqlite()`
/// defined in their respective crates.
#[derive(Clone)]
pub struct DbContextOptions {
    /// Database connection string.
    pub(crate) connection_string: String,
    /// Provider identifier tag (e.g. "sqlite", "postgres", "mysql").
    pub(crate) provider_tag: Option<String>,
    /// Optional pre-built provider instance (for advanced scenarios).
    pub(crate) provider_instance: Option<Arc<dyn IDatabaseProvider>>,
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

    pub fn provider_instance(&self) -> Option<&Arc<dyn IDatabaseProvider>> {
        self.provider_instance.as_ref()
    }
}

impl Default for DbContextOptions {
    fn default() -> Self {
        Self {
            connection_string: String::new(),
            provider_tag: None,
            provider_instance: None,
        }
    }
}

/// Fluent builder for `DbContextOptions`.
///
/// Provider crates extend this with methods like `use_sqlite()` via trait
/// extensions defined in their respective crates.
///
/// # Extension Traits
///
/// | Provider | Extension Trait | Method |
/// |---|---|---|
/// | `lref-provider-sqlite`    | `DbContextOptionsBuilderExt` | `.use_sqlite(cs)` |
/// | `lref-provider-postgres`  | `DbContextOptionsBuilderExt` | `.use_postgres(cs)` |
/// | `lref-provider-mysql`     | `DbContextOptionsBuilderExt` | `.use_mysql(cs)` |
pub struct DbContextOptionsBuilder {
    inner: DbContextOptions,
}

impl DbContextOptionsBuilder {
    pub fn new() -> Self {
        Self {
            inner: DbContextOptions::default(),
        }
    }

    /// Sets the connection string (without specifying a provider).
    pub fn connection_string(&mut self, cs: impl Into<String>) -> &mut Self {
        self.inner.connection_string = cs.into();
        self
    }

    /// Sets the provider tag and connection string.
    /// Called by provider extension methods.
    pub fn set_provider(&mut self, tag: &str, connection_string: impl Into<String>) -> &mut Self {
        self.inner.provider_tag = Some(tag.to_string());
        self.inner.connection_string = connection_string.into();
        self
    }

    /// Attaches a pre-built provider instance (advanced usage).
    pub fn use_provider_instance(
        &mut self,
        tag: &str,
        provider: Arc<dyn IDatabaseProvider>,
    ) -> &mut Self {
        self.inner.provider_tag = Some(tag.to_string());
        self.inner.provider_instance = Some(provider);
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
// IDbContext trait
// ---------------------------------------------------------------------------

/// The DbContext trait represents a session with the database.
#[async_trait::async_trait]
pub trait IDbContext: Send + Sync + Sized {
    type Provider: crate::provider::IDatabaseProvider;

    /// Constructs the context from `DbContextOptions` and a service resolver.
    ///
    /// The `services` parameter provides access to the DI container, allowing
    /// the context to resolve additional services (loggers, interceptors, etc.)
    /// during construction.
    fn from_options(
        options: &DbContextOptions,
        services: &dyn lrdi::IServiceResolver,
    ) -> LrefResult<Self>;

    fn provider(&self) -> &Self::Provider;
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker;
    fn change_tracker(&self) -> &ChangeTracker;
    async fn save_changes(&mut self) -> LrefResult<SaveChangesResult>;

    async fn begin_transaction(&self) -> LrefResult<Box<dyn IAsyncConnection>> {
        let mut conn: Box<dyn IAsyncConnection> = self.provider().get_connection().await?;
        conn.begin_transaction().await?;
        Ok(conn)
    }

    async fn use_transaction<F, Fut, R>(&self, f: F) -> LrefResult<R>
    where
        F: FnOnce(&mut dyn IAsyncConnection) -> Fut + Send,
        Fut: std::future::Future<Output = LrefResult<R>> + Send,
        R: Send,
    {
        let mut conn: Box<dyn IAsyncConnection> = self.provider().get_connection().await?;
        conn.begin_transaction().await?;
        match f(&mut *conn).await {
            Ok(result) => {
                conn.commit_transaction().await?;
                Ok(result)
            }
            Err(e) => {
                let _ = conn.rollback_transaction().await;
                Err(e)
            }
        }
    }

    fn set_logging(&mut self, _enabled: bool) {}
    fn is_logging_enabled(&self) -> bool {
        false
    }
    #[allow(unused_variables)]
    fn log_sql(&self, sql: &str, params_count: usize) {}

    async fn ensure_created(&self) -> LrefResult<()> {
        let conn_str = format!("{}", self.provider().name());
        let _ = conn_str;
        Err(crate::error::LrefError::Configuration(
            "ensure_created requires entity metadata. Use migration engine instead.".into(),
        ))
    }

    async fn ensure_deleted(&self) -> LrefResult<()> {
        Err(crate::error::LrefError::Configuration(
            "ensure_deleted requires entity metadata. Use migration engine instead.".into(),
        ))
    }
}

/// Saves changes for one entity type within an active transaction.
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

    let mut added_count = 0usize;
    let mut updated_count = 0usize;
    let mut deleted_count = 0usize;

    if !added.is_empty() {
        added_count = ChangeExecutor::execute_inserts(conn, provider, &added, |_, _| {}).await?;
    }
    if !modified.is_empty() {
        updated_count = ChangeExecutor::execute_updates(conn, provider, &modified).await?;
    }
    if !deleted.is_empty() {
        deleted_count = ChangeExecutor::execute_deletes(conn, provider, &deleted).await?;
    }

    Ok((added_count, updated_count, deleted_count))
}

/// Macro to save changes for multiple entity types in a single transaction.
#[macro_export]
macro_rules! save_changes_all {
    ($ctx:expr, $first:expr $(, $rest:expr)* $(,)?) => {{
        $ctx.change_tracker_mut().detect_changes();
        let mut conn = $ctx.provider().get_connection().await?;
        conn.begin_transaction().await?;

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut deleted = 0usize;

        {
            let (a, u, d) = $crate::db_context::save_one_set(
                &mut *conn, $ctx.provider(), &mut $first
            ).await?;
            added += a; updated += u; deleted += d;
        }
        $(
            {
                let (a, u, d) = $crate::db_context::save_one_set(
                    &mut *conn, $ctx.provider(), &mut $rest
                ).await?;
                added += a; updated += u; deleted += d;
            }
        )*

        conn.commit_transaction().await?;
        $ctx.change_tracker_mut().accept_all_changes();
        $first.clear_entries();
        $($rest.clear_entries();)*

        Ok($crate::db_context::SaveChangesResult { added, updated, deleted })
    }};
}

/// Result of calling save_changes().
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
