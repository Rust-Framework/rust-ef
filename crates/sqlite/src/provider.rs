use crate::sql_generator::SqliteSqlGenerator;
use async_trait::async_trait;
use r2d2::CustomizeConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{IDatabaseProvider, ISqlGenerator};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Initializes each pooled connection with WAL mode and a busy timeout.
///
/// WAL allows concurrent readers while a writer holds the lock; the busy
/// timeout makes writers wait up to 5 seconds for `SQLITE_BUSY` to clear
/// instead of failing immediately. Applied to every connection at acquisition
/// time so pool growth doesn't lose the PRAGMAs.
#[derive(Debug)]
struct SqliteConnectionCustomizer;

impl CustomizeConnection<Connection, rusqlite::Error> for SqliteConnectionCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
    }
}

/// Default max pool size for file-based SQLite databases.
const SQLITE_DEFAULT_POOL_SIZE: u32 = 8;

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
enum SqliteProviderInner {
    Pooled(r2d2::Pool<SqliteConnectionManager>),
    Single(Arc<Mutex<rusqlite::Connection>>),
}

impl SqliteProviderInner {
    async fn get_connection(&self) -> EFResult<Box<dyn rust_ef::provider::IAsyncConnection>> {
        match self {
            SqliteProviderInner::Pooled(pool) => {
                let conn = pool.get().map_err(|e| {
                    EFError::Connection(format!("SQLite pool acquire failed: {}", e))
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

    async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
        match self {
            SqliteProviderInner::Pooled(pool) => {
                let conn = pool.get().map_err(|e| {
                    EFError::Connection(format!("SQLite pool acquire failed: {}", e))
                })?;
                conn.execute_batch(sql).map_err(|e| {
                    EFError::Migration(format!("Migration execution failed: {}", e))
                })?;
                Ok(())
            }
            SqliteProviderInner::Single(conn) => {
                let guard = conn.lock().await;
                guard.execute_batch(sql).map_err(|e| {
                    EFError::Migration(format!("Migration execution failed: {}", e))
                })?;
                Ok(())
            }
        }
    }
}

pub struct SqliteProvider {
    inner: SqliteProviderInner,
}

impl SqliteProvider {
    /// Creates a provider for a file-based SQLite database with a connection
    /// pool (default 8 connections). WAL mode and a 5s busy timeout are
    /// applied to every pooled connection.
    pub fn new(path: impl AsRef<std::path::Path>) -> EFResult<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = r2d2::Pool::builder()
            .max_size(SQLITE_DEFAULT_POOL_SIZE)
            .connection_customizer(Box::new(SqliteConnectionCustomizer))
            .build(manager)
            .map_err(|e| EFError::Connection(format!("SQLite pool creation failed: {}", e)))?;
        Ok(Self {
            inner: SqliteProviderInner::Pooled(pool),
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
            .map_err(|e| EFError::Connection(format!("SQLite in-memory open failed: {}", e)))?;
        // WAL isn't supported for in-memory databases (SQLite silently
        // ignores the PRAGMA), but busy_timeout still applies.
        conn.execute_batch("PRAGMA busy_timeout=5000;")
            .map_err(|e| EFError::Connection(format!("SQLite pragma setup failed: {}", e)))?;
        Ok(Self {
            inner: SqliteProviderInner::Single(Arc::new(Mutex::new(conn))),
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
        self.inner.get_connection().await
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
}

impl std::fmt::Debug for SqliteProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteProvider")
            .field("name", &self.name())
            .finish()
    }
}
