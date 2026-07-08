use async_trait::async_trait;
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex as TokioMutex;

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
}

impl SqliteConnection {
    pub(crate) fn new_pooled(conn: PooledConnection<SqliteConnectionManager>) -> Self {
        Self {
            inner: SqliteConnectionInner::Pooled(StdMutex::new(conn)),
        }
    }

    pub(crate) fn new_shared(conn: Arc<TokioMutex<rusqlite::Connection>>) -> Self {
        Self {
            inner: SqliteConnectionInner::Shared(conn),
        }
    }
}

// ---------------------------------------------------------------------------
// Synchronous core logic — shared by both connection modes
// ---------------------------------------------------------------------------

fn execute_sync(conn: &rusqlite::Connection, sql: &str, params: &[DbValue]) -> EFResult<u64> {
    let rp = crate::type_conversion::to_rusqlite_params(params);
    let refs: Vec<&dyn rusqlite::types::ToSql> = rp
        .iter()
        .map(|v| v as &dyn rusqlite::types::ToSql)
        .collect();
    conn.execute(sql, refs.as_slice())
        .map(|c| c as u64)
        .map_err(|e| EFError::Query(format!("Execution error: {}", e)))
}

fn query_sync(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[DbValue],
) -> EFResult<Vec<Vec<String>>> {
    let rp = crate::type_conversion::to_rusqlite_params(params);
    let refs: Vec<&dyn rusqlite::types::ToSql> = rp
        .iter()
        .map(|v| v as &dyn rusqlite::types::ToSql)
        .collect();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| EFError::Query(format!("Prepare error: {}", e)))?;
    let cc = stmt.column_count();
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            let mut vals = Vec::with_capacity(cc);
            for i in 0..cc {
                vals.push(
                    row.get::<_, String>(i)
                        .or_else(|_| row.get::<_, i64>(i).map(|n| n.to_string()))
                        .or_else(|_| row.get::<_, f64>(i).map(|n| n.to_string()))
                        .unwrap_or_else(|_| "NULL".to_string()),
                );
            }
            Ok(vals)
        })
        .map_err(|e| EFError::Query(format!("Query error: {}", e)))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| EFError::Query(format!("Row read error: {}", e)))?);
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// IAsyncConnection impl — dispatches to the sync core via the right lock
// ---------------------------------------------------------------------------

#[async_trait]
impl IAsyncConnection for SqliteConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
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
}
