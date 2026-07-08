//! SQLite connection management strategy.
//!
//! Separates pool/memory dispatch from the public provider so `provider.rs`
//! only contains `SqliteProvider` and its trait impls.
use r2d2::CustomizeConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use rust_ef::error::{EFError, EFResult};
use std::sync::Arc;
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
    ) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        match self {
            SqliteProviderInner::Pooled(pool) => {
                let conn = pool.get().map_err(|e| {
                    EFError::connection(format!("SQLite pool acquire failed: {}", e))
                })?;
                Ok(Box::new(crate::connection::SqliteConnection::new_pooled(
                    conn,
                )))
            }
            SqliteProviderInner::Single(conn) => Ok(Box::new(
                crate::connection::SqliteConnection::new_shared(Arc::clone(conn)),
            )),
        }
    }

    pub(crate) async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
        match self {
            SqliteProviderInner::Pooled(pool) => {
                let conn = pool.get().map_err(|e| {
                    EFError::connection(format!("SQLite pool acquire failed: {}", e))
                })?;
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
