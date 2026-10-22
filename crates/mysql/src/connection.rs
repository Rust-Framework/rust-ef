use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use sqlx::{Executor, Row};

pub struct MySqlConnection {
    conn: Option<sqlx::pool::PoolConnection<sqlx::MySql>>,
    #[cfg(feature = "tracing")]
    slow_query_threshold: Option<std::time::Duration>,
}

fn isolation_sql(level: IsolationLevel) -> &'static str {
    match level {
        IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
        IsolationLevel::ReadCommitted => "READ COMMITTED",
        IsolationLevel::RepeatableRead => "REPEATABLE READ",
        IsolationLevel::Serializable => "SERIALIZABLE",
    }
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

    /// Executes a statement through MySQL's text protocol.
    ///
    /// Transaction control and DDL statements are not supported by the
    /// prepared-statement protocol (MySQL error 1295), so they must use
    /// `raw_sql` instead of `sqlx::query`.
    async fn execute_raw(&mut self, sql: &str) -> EFResult<u64> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| EFError::connection("Connection already closed".to_string()))?;
        let result = conn
            .execute(sqlx::raw_sql(sql))
            .await
            .map_err(|e| EFError::query(format!("Execution error: {}", e)))?;
        Ok(result.rows_affected())
    }
}

#[async_trait]
impl IAsyncConnection for MySqlConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| EFError::connection("Connection already closed".to_string()))?;
        let result = crate::type_conversion::build_mysql_query(sql, params)
            .execute(&mut **conn)
            .await
            .map_err(|e| EFError::query(format!("Execution error: {}", e)))?;
        Ok(result.rows_affected())
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<DbValue>>> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| EFError::connection("Connection already closed".to_string()))?;
        let rows = crate::type_conversion::build_mysql_query(sql, params)
            .fetch_all(&mut **conn)
            .await
            .map_err(|e| EFError::query(format!("Query error: {}", e)))?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let result = rows
            .iter()
            .map(|row| {
                row.columns()
                    .iter()
                    .enumerate()
                    .map(|(i, _)| crate::row_conversion::cell_to_db_value(row, i))
                    .collect()
            })
            .collect();

        Ok(result)
    }

    async fn begin_transaction(&mut self) -> EFResult<()> {
        self.execute_raw("START TRANSACTION").await.map(|_| ())
    }

    async fn begin_transaction_with(&mut self, level: IsolationLevel) -> EFResult<()> {
        // MySQL only accepts SET TRANSACTION before the transaction starts.
        let sql = format!("SET TRANSACTION ISOLATION LEVEL {}", isolation_sql(level));
        self.execute_raw(&sql).await?;
        self.begin_transaction().await
    }

    async fn commit_transaction(&mut self) -> EFResult<()> {
        self.execute_raw("COMMIT").await.map(|_| ())
    }

    async fn rollback_transaction(&mut self) -> EFResult<()> {
        self.execute_raw("ROLLBACK").await.map(|_| ())
    }

    async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.execute_raw(&format!("SAVEPOINT {}", name))
            .await
            .map(|_| ())
    }

    async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.execute_raw(&format!("RELEASE SAVEPOINT {}", name))
            .await
            .map(|_| ())
    }

    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.execute_raw(&format!("ROLLBACK TO SAVEPOINT {}", name))
            .await
            .map(|_| ())
    }

    async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
        // Legacy post-BEGIN API: keep the historical SQL. MySQL only applies
        // it reliably before START TRANSACTION; use `begin_transaction_with`
        // for the portable entry point.
        let sql = format!("SET TRANSACTION ISOLATION LEVEL {}", isolation_sql(level));
        self.execute_raw(&sql).await.map(|_| ())
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&mut self, threshold: std::time::Duration) {
        self.slow_query_threshold = Some(threshold);
    }

    fn discard(self: Box<Self>) {
        let this = *self;
        if let Some(conn) = this.conn {
            // Detach from the pool; dropping the underlying connection closes
            // it, and MySQL rolls back any open transaction on disconnect.
            let _ = conn.detach();
        }
    }
}
