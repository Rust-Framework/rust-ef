use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SqliteConnection {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteConnection {
    pub(crate) fn new(conn: Arc<Mutex<rusqlite::Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl IAsyncConnection for SqliteConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        let conn = self.conn.lock().await;
        let rp = crate::type_conversion::to_rusqlite_params(params);
        let refs: Vec<&dyn rusqlite::types::ToSql> = rp
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        conn.execute(sql, refs.as_slice())
            .map(|c| c as u64)
            .map_err(|e| EFError::Query(format!("Execution error: {}", e)))
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>> {
        let conn = self.conn.lock().await;
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
