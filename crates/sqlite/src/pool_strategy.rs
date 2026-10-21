//! SQLite connection management strategy.
//!
//! Separates pool/memory dispatch from the public provider so `provider.rs`
//! only contains `SqliteProvider` and its trait impls.
use r2d2::CustomizeConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use rust_ef::error::{EFError, EFResult};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Initializes each pooled connection with WAL mode and a busy timeout.
///
/// WAL allows concurrent readers while a writer holds the lock; the busy
/// timeout makes writers wait up to 5 seconds for `SQLITE_BUSY` to clear
/// instead of failing immediately. Applied to every connection at acquisition
/// time so pool growth doesn't lose the PRAGMAs.
#[derive(Debug)]
pub(crate) struct SqliteConnectionCustomizer;

impl CustomizeConnection<Connection, rusqlite::Error> for SqliteConnectionCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
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
                conn.execute_batch(sql).map_err(|e| {
                    EFError::migration(format!("Migration execution failed: {}", e))
                })?;
                Ok(())
            }
            SqliteProviderInner::Single(conn) => {
                let guard = conn.lock().await;
                guard.execute_batch(sql).map_err(|e| {
                    EFError::migration(format!("Migration execution failed: {}", e))
                })?;
                Ok(())
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
