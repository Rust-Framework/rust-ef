use crate::pool_strategy::{
    SqliteConnectionCustomizer, SqliteProviderInner, SQLITE_DEFAULT_POOL_ACQUIRE_TIMEOUT,
    SQLITE_DEFAULT_POOL_SIZE,
};
use crate::sql_generator::SqliteSqlGenerator;
use async_trait::async_trait;
use r2d2_sqlite::SqliteConnectionManager;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Options for a file-based SQLite provider.
///
/// Configured through the `use_sqlite_with(path, |options| { ... })` delegate;
/// new SQLite capabilities (read-only mode, busy timeout, journal mode, …) can
/// be added here without growing the number of `use_sqlite_*` methods.
#[derive(Clone, Debug)]
pub struct SqliteOptions {
    pub(crate) path: String,
    pub(crate) max_pool_size: u32,
    pub(crate) pool_acquire_timeout: Duration,
}

impl SqliteOptions {
    pub(crate) fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            max_pool_size: SQLITE_DEFAULT_POOL_SIZE,
            pool_acquire_timeout: SQLITE_DEFAULT_POOL_ACQUIRE_TIMEOUT,
        }
    }

    /// Sets the maximum number of pooled connections.
    ///
    /// Use `1` for the write context in a read/write split; the read context can
    /// keep the default.
    pub fn max_pool_size(&mut self, max_pool_size: u32) -> &mut Self {
        self.max_pool_size = max_pool_size;
        self
    }

    /// Sets how long to wait for a pooled connection before returning
    /// `ConnectionTimeout`.
    pub fn pool_acquire_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.pool_acquire_timeout = timeout;
        self
    }
}

pub struct SqliteProvider {
    inner: SqliteProviderInner,
    acquire_timeout: Duration,
    #[cfg(feature = "tracing")]
    slow_query_threshold_ms: std::sync::atomic::AtomicU64,
}

impl SqliteProvider {
    /// Creates a provider for a file-based SQLite database with a connection
    /// pool (default 8 connections). WAL mode and a 5s busy timeout are
    /// applied to every pooled connection.
    ///
    /// Note: `:memory:` (without URI) is special-cased to delegate to
    /// [`new_in_memory`](Self::new_in_memory) because SQLite `:memory:`
    /// databases are per-connection — a pool would give each connection its
    /// own isolated database. URI-form shared in-memory DBs (e.g.
    /// `file::memory:?cache=shared`) intentionally use `Pooled` mode.
    pub fn new(path: impl AsRef<std::path::Path>) -> EFResult<Self> {
        let path_ref = path.as_ref();
        if path_ref == std::path::Path::new(":memory:") {
            return Self::new_in_memory();
        }
        Self::new_with_pool(path_ref, SQLITE_DEFAULT_POOL_SIZE)
    }

    /// Creates a file-based provider with an explicit pool size.
    ///
    /// For read/write separation, the write context should use `max_size = 1`
    /// so writes are serialized inside the framework, while the read context
    /// keeps the default pool. `:memory:` is rejected: each connection would
    /// get its own isolated database.
    pub fn new_with_pool(path: impl AsRef<std::path::Path>, max_size: u32) -> EFResult<Self> {
        Self::build_pool(path.as_ref(), max_size, SQLITE_DEFAULT_POOL_ACQUIRE_TIMEOUT)
    }

    /// Creates a file-based provider from [`SqliteOptions`].
    pub(crate) fn new_with_options(options: &SqliteOptions) -> EFResult<Self> {
        Self::build_pool(
            std::path::Path::new(&options.path),
            options.max_pool_size,
            options.pool_acquire_timeout,
        )
    }

    fn build_pool(
        path: &std::path::Path,
        max_size: u32,
        acquire_timeout: Duration,
    ) -> EFResult<Self> {
        if path == std::path::Path::new(":memory:") {
            return Err(EFError::configuration(
                "use_sqlite_with does not support :memory:; use use_sqlite_in_memory",
            ));
        }
        if max_size == 0 {
            return Err(EFError::configuration(
                "SQLite pool max_size must be greater than 0",
            ));
        }
        if acquire_timeout.is_zero() {
            return Err(EFError::configuration(
                "SQLite pool acquire timeout must be greater than 0",
            ));
        }
        let manager = SqliteConnectionManager::file(path);
        let pool = r2d2::Pool::builder()
            .max_size(max_size)
            .connection_timeout(acquire_timeout)
            .connection_customizer(Box::new(SqliteConnectionCustomizer))
            .build(manager)
            .map_err(|e| EFError::connection(format!("SQLite pool creation failed: {}", e)))?;
        Ok(Self {
            inner: SqliteProviderInner::Pooled(pool),
            acquire_timeout,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Creates a provider for an in-memory SQLite database.
    ///
    /// Uses a single shared connection (`Arc<Mutex<Connection>>`) because
    /// SQLite `:memory:` databases are per-connection — a pool would give
    /// each connection its own isolated database. This preserves the
    /// pre-v1.4 behavior: each `new_in_memory()` call creates a fresh,
    /// independent database with full test isolation.
    pub fn new_in_memory() -> EFResult<Self> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| EFError::connection(format!("SQLite in-memory open failed: {}", e)))?;
        // WAL isn't supported for in-memory databases (SQLite silently
        // ignores the PRAGMA), but busy_timeout still applies.
        conn.execute_batch("PRAGMA busy_timeout=5000;")
            .map_err(|e| EFError::connection(format!("SQLite pragma setup failed: {}", e)))?;
        Ok(Self {
            inner: SqliteProviderInner::Single(Arc::new(Mutex::new(conn))),
            acquire_timeout: SQLITE_DEFAULT_POOL_ACQUIRE_TIMEOUT,
            #[cfg(feature = "tracing")]
            slow_query_threshold_ms: std::sync::atomic::AtomicU64::new(0),
        })
    }
}

#[async_trait]
impl IDatabaseProvider for SqliteProvider {
    fn sql_generator(&self) -> &'static dyn ISqlGenerator {
        // Stateless generator: rvalue static promotion gives `&'static`.
        &SqliteSqlGenerator
    }

    async fn get_connection(&self) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        let _guard = rust_ef::observability::PoolAcquireGuard::new("SQLite");
        #[cfg_attr(not(feature = "tracing"), allow(unused_mut))]
        let mut conn = self.inner.get_connection(self.acquire_timeout).await?;
        #[cfg(feature = "tracing")]
        {
            let ms = self
                .slow_query_threshold_ms
                .load(std::sync::atomic::Ordering::Relaxed);
            if ms > 0 {
                conn.set_slow_query_threshold(std::time::Duration::from_millis(ms));
            }
        }
        Ok(conn)
    }

    async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
        self.inner
            .execute_migration_command(sql, self.acquire_timeout)
            .await
    }

    fn name(&self) -> &str {
        "SQLite"
    }

    fn migration_dialect(&self) -> rust_ef::migration::MigrationDialect {
        rust_ef::migration::MigrationDialect::Sqlite
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&self, threshold: std::time::Duration) {
        self.slow_query_threshold_ms.store(
            threshold.as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl std::fmt::Debug for SqliteProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteProvider")
            .field("name", &self.name())
            .finish()
    }
}
