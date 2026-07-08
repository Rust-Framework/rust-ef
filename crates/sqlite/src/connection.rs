use async_trait::async_trait;
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rust_ef::error::EFResult;
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex as TokioMutex;

use crate::sync_ops::{execute_sync, query_sync};

/// The underlying SQLite connection handle.
///
/// `Pooled` wraps an `r2d2::PooledConnection` (used for file-based databases
/// with a connection pool). `Shared` wraps an `Arc<tokio::sync::Mutex<Connection>>`
/// (used for `:memory:` databases, which must share a single connection).
enum SqliteConnectionInner {
    Pooled(StdMutex<PooledConnection<SqliteConnectionManager>>),
    Shared(Arc<TokioMutex<rusqlite::Connection>>),
}

pub struct SqliteConnection {
    inner: SqliteConnectionInner,
    #[cfg(feature = "tracing")]
    slow_query_threshold: Option<std::time::Duration>,
}

impl SqliteConnection {
    pub(crate) fn new_pooled(conn: PooledConnection<SqliteConnectionManager>) -> Self {
        Self {
            inner: SqliteConnectionInner::Pooled(StdMutex::new(conn)),
            #[cfg(feature = "tracing")]
            slow_query_threshold: None,
        }
    }

    pub(crate) fn new_shared(conn: Arc<TokioMutex<rusqlite::Connection>>) -> Self {
        Self {
            inner: SqliteConnectionInner::Shared(conn),
            #[cfg(feature = "tracing")]
            slow_query_threshold: None,
        }
    }

    fn threshold(&self) -> Option<std::time::Duration> {
        #[cfg(feature = "tracing")]
        {
            self.slow_query_threshold
        }
        #[cfg(not(feature = "tracing"))]
        {
            None
        }
    }
}

#[async_trait]
impl IAsyncConnection for SqliteConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        match &self.inner {
            SqliteConnectionInner::Pooled(m) => {
                // &mut self guarantees no contention; poison is recovered
                // rather than propagated.
                let conn = m.lock().unwrap_or_else(|p| p.into_inner());
                execute_sync(&conn, sql, params)
            }
            SqliteConnectionInner::Shared(m) => {
                let conn = m.lock().await;
                execute_sync(&conn, sql, params)
            }
        }
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        match &self.inner {
            SqliteConnectionInner::Pooled(m) => {
                let conn = m.lock().unwrap_or_else(|p| p.into_inner());
                query_sync(&conn, sql, params)
            }
            SqliteConnectionInner::Shared(m) => {
                let conn = m.lock().await;
                query_sync(&conn, sql, params)
            }
        }
    }

    async fn begin_transaction(&mut self) -> EFResult<()> {
        self.execute("BEGIN TRANSACTION", &[]).await.map(|_| ())
    }

    async fn commit_transaction(&mut self) -> EFResult<()> {
        self.execute("COMMIT", &[]).await.map(|_| ())
    }

    async fn rollback_transaction(&mut self) -> EFResult<()> {
        self.execute("ROLLBACK", &[]).await.map(|_| ())
    }

    async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.execute(&format!("SAVEPOINT {}", name), &[])
            .await
            .map(|_| ())
    }

    async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.execute(&format!("RELEASE {}", name), &[])
            .await
            .map(|_| ())
    }

    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.execute(&format!("ROLLBACK TO {}", name), &[])
            .await
            .map(|_| ())
    }

    async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
        let sql = match level {
            IsolationLevel::ReadUncommitted => "PRAGMA read_uncommitted = ON",
            _ => "PRAGMA read_uncommitted = OFF",
        };
        self.execute(sql, &[]).await.map(|_| ())
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&mut self, threshold: std::time::Duration) {
        self.slow_query_threshold = Some(threshold);
    }
}
