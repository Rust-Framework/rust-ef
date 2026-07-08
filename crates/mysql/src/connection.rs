use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use sqlx::Row;

pub struct MySqlConnection {
    conn: Option<sqlx::pool::PoolConnection<sqlx::MySql>>,
    #[cfg(feature = "tracing")]
    slow_query_threshold: Option<std::time::Duration>,
}

impl MySqlConnection {
    pub(crate) fn new(conn: sqlx::pool::PoolConnection<sqlx::MySql>) -> Self {
        Self {
            conn: Some(conn),
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
impl IAsyncConnection for MySqlConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| EFError::Connection("Connection already closed".to_string()))?;
        let result = crate::type_conversion::build_mysql_query(sql, params)
            .execute(&mut **conn)
            .await
            .map_err(|e| EFError::Query(format!("Execution error: {}", e)))?;
        Ok(result.rows_affected())
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| EFError::Connection("Connection already closed".to_string()))?;
        let rows = crate::type_conversion::build_mysql_query(sql, params)
            .fetch_all(&mut **conn)
            .await
            .map_err(|e| EFError::Query(format!("Query error: {}", e)))?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let result = rows
            .iter()
            .map(|row| {
                row.columns()
                    .iter()
                    .enumerate()
                    .map(|(i, _)| crate::row_conversion::cell_to_string(row, i))
                    .collect()
            })
            .collect();

        Ok(result)
    }

    async fn begin_transaction(&mut self) -> EFResult<()> {
        self.execute("START TRANSACTION", &[]).await.map(|_| ())
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
        self.execute(&format!("RELEASE SAVEPOINT {}", name), &[])
            .await
            .map(|_| ())
    }

    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.execute(&format!("ROLLBACK TO SAVEPOINT {}", name), &[])
            .await
            .map(|_| ())
    }

    async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
        let sql = format!(
            "SET TRANSACTION ISOLATION LEVEL {}",
            match level {
                IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
                IsolationLevel::ReadCommitted => "READ COMMITTED",
                IsolationLevel::RepeatableRead => "REPEATABLE READ",
                IsolationLevel::Serializable => "SERIALIZABLE",
            }
        );
        self.execute(&sql, &[]).await.map(|_| ())
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&mut self, threshold: std::time::Duration) {
        self.slow_query_threshold = Some(threshold);
    }
}
