//! SQLite connection management strategy.
//!
//! Separates pool/memory dispatch from the public provider so `provider.rs`
//! only contains `SqliteProvider` and its trait impls.
use r2d2::CustomizeConnection;
use rusqlite::Connection;
use rust_ef::error::{EFError, EFResult};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// r2d2 connection manager for file-based SQLite databases.
///
/// This replaces `r2d2_sqlite`, whose latest release still pins rusqlite 0.40
/// (libsqlite3-sys 0.38, affected by RUSTSEC-2026-0459). A manager only needs
/// to open the connection; WAL and busy_timeout are applied by
/// [`SqliteConnectionCustomizer`].
#[derive(Debug, Clone)]
pub(crate) struct SqliteConnectionManager {
    path: PathBuf,
}

impl SqliteConnectionManager {
    pub(crate) fn file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[cfg(test)]
impl SqliteConnectionManager {
    /// Pooled in-memory manager used only to exercise pool acquisition. Every
    /// pooled connection gets its own database, which is fine for a timeout
    /// test that never queries data.
    pub(crate) fn memory() -> Self {
        Self {
            path: PathBuf::from(":memory:"),
        }
    }
}

impl r2d2::ManageConnection for SqliteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        Connection::open(&self.path)
    }

    fn is_valid(&self, conn: &mut Self::Connection) -> Result<(), Self::Error> {
        // Matches r2d2_sqlite: an empty batch checks the handle without
        // issuing a query on every pool checkout.
        conn.execute_batch("")
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

/// Initializes each pooled connection with WAL mode and a busy timeout.
///
/// WAL allows concurrent readers while a writer holds the lock; the busy
/// timeout makes writers wait up to 5 seconds for `SQLITE_BUSY` to clear
/// instead of failing immediately. Applied to every connection at acquisition
/// time so pool growth doesn't lose the PRAGMAs.
///
/// When `foreign_keys` is enabled, `PRAGMA foreign_keys=ON` runs on the same
/// acquisition hook: SQLite's foreign-key enforcement is a *connection-level*
/// switch, so it must be re-applied to every connection the pool hands out.
#[derive(Debug)]
pub(crate) struct SqliteConnectionCustomizer {
    pub(crate) foreign_keys: bool,
}

/// SQLite PRAGMAs applied when a connection is acquired/opened.
///
/// Kept as a single function so the pooled (`r2d2` customizer) and the
/// in-memory (shared connection) paths cannot drift apart.
pub(crate) fn connection_init_sql(foreign_keys: bool) -> &'static str {
    if foreign_keys {
        "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;"
    } else {
        "PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;"
    }
}

impl CustomizeConnection<Connection, rusqlite::Error> for SqliteConnectionCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(connection_init_sql(self.foreign_keys))
    }
}

/// Default max pool size for file-based SQLite databases.
pub(crate) const SQLITE_DEFAULT_POOL_SIZE: u32 = 8;

/// Maximum time to wait for a pooled connection before returning
/// [`EFErrorCode::ConnectionTimeout`](rust_ef::error::EFErrorCode::ConnectionTimeout).
///
/// Matches r2d2's historical default so existing `use_sqlite` callers keep the
/// previous timeout behavior; `SqliteOptions::pool_acquire_timeout` can shorten
/// it for latency-sensitive write contexts.
pub(crate) const SQLITE_DEFAULT_POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Acquires a pooled SQLite connection without blocking the async runtime.
///
/// `r2d2::Pool::get_timeout` is blocking; running it on `spawn_blocking` keeps
/// a saturated write pool (`max_size = 1`) from stalling Tokio worker threads.
pub(crate) async fn acquire_pooled(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    timeout: Duration,
) -> EFResult<r2d2::PooledConnection<SqliteConnectionManager>> {
    acquire_pooled_with_timeout(pool, timeout).await
}

async fn acquire_pooled_with_timeout(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    timeout: Duration,
) -> EFResult<r2d2::PooledConnection<SqliteConnectionManager>> {
    let pool = pool.clone();
    let result = tokio::task::spawn_blocking(move || pool.get_timeout(timeout))
        .await
        .map_err(|e| EFError::connection(format!("SQLite pool acquire task failed: {}", e)))?;
    result.map_err(|e| {
        EFError::connection(format!(
            "SQLite pool acquire timeout after {}ms: {}",
            timeout.as_millis(),
            e
        ))
    })
}

/// The connection management strategy.
///
/// `Pooled` is used for file-based databases — r2d2 maintains a pool of
/// connections that can be checked out concurrently.
///
/// `Single` is used for `:memory:` databases — SQLite `:memory:` databases
/// are per-connection, so a single shared connection (behind a `Mutex`) is
/// the only way to keep all operations on the same database. This matches
/// the pre-v1.4 behavior and preserves test isolation (each
/// `new_in_memory()` call gets a fresh, independent database).
pub(crate) enum SqliteProviderInner {
    Pooled(r2d2::Pool<SqliteConnectionManager>),
    Single(Arc<Mutex<rusqlite::Connection>>),
}

impl SqliteProviderInner {
    pub(crate) async fn get_connection(
        &self,
        timeout: Duration,
    ) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        match self {
            SqliteProviderInner::Pooled(pool) => {
                let conn = acquire_pooled(pool, timeout).await?;
                Ok(Box::new(crate::connection::SqliteConnection::new_pooled(
                    conn,
                )))
            }
            SqliteProviderInner::Single(conn) => Ok(Box::new(
                crate::connection::SqliteConnection::new_shared(Arc::clone(conn)),
            )),
        }
    }

    pub(crate) async fn execute_migration_command(
        &self,
        sql: &str,
        timeout: Duration,
    ) -> EFResult<()> {
        match self {
            SqliteProviderInner::Pooled(pool) => {
                let conn = acquire_pooled(pool, timeout).await?;
                conn.execute_batch(sql)
                    .map_err(|e| crate::sync_ops::map_sqlite_error_with_sql(e, Some(sql)))?;
                Ok(())
            }
            SqliteProviderInner::Single(conn) => {
                let guard = conn.lock().await;
                guard
                    .execute_batch(sql)
                    .map_err(|e| crate::sync_ops::map_sqlite_error_with_sql(e, Some(sql)))?;
                Ok(())
            }
        }
    }

    /// Reads the live schema. Pooled providers borrow one connection; the
    /// in-memory provider introspects through its shared connection.
    pub(crate) async fn introspect(
        &self,
        timeout: Duration,
    ) -> EFResult<Vec<rust_ef::schema::DbTable>> {
        match self {
            SqliteProviderInner::Pooled(pool) => {
                let conn = acquire_pooled(pool, timeout).await?;
                // PRAGMA/`sqlite_master` reads are synchronous; keep them off
                // the async runtime like the execute/query paths do.
                tokio::task::spawn_blocking(move || {
                    crate::introspection::introspect_connection(&conn)
                })
                .await
                .map_err(|e| EFError::query(format!("SQLite introspection task failed: {}", e)))?
            }
            SqliteProviderInner::Single(conn) => {
                let guard = conn.lock().await;
                crate::introspection::introspect_connection(&guard)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_ef::error::EFErrorCode;

    #[tokio::test]
    async fn pool_acquire_timeout_is_reported_as_connection_timeout() {
        let manager = SqliteConnectionManager::memory();
        let pool = r2d2::Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("pool");

        let _held = pool.get().expect("first connection");
        let err = acquire_pooled_with_timeout(&pool, Duration::from_millis(50))
            .await
            .expect_err("second acquire must time out");

        assert!(err.message().contains("timeout"), "got: {err}");
        assert_eq!(err.code(), EFErrorCode::ConnectionTimeout);
    }
}
