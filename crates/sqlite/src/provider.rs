use crate::pool_strategy::{
    SqliteConnectionCustomizer, SqliteProviderInner, SQLITE_DEFAULT_POOL_SIZE,
};
use crate::sql_generator::SqliteSqlGenerator;
use async_trait::async_trait;
use r2d2_sqlite::SqliteConnectionManager;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SqliteProvider {
    inner: SqliteProviderInner,
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
        let manager = SqliteConnectionManager::file(path_ref);
        let pool = r2d2::Pool::builder()
            .max_size(SQLITE_DEFAULT_POOL_SIZE)
            .connection_customizer(Box::new(SqliteConnectionCustomizer))
            .build(manager)
            .map_err(|e| EFError::connection(format!("SQLite pool creation failed: {}", e)))?;
        Ok(Self {
            inner: SqliteProviderInner::Pooled(pool),
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
        let mut conn = self.inner.get_connection().await?;
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
        self.inner.execute_migration_command(sql).await
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
