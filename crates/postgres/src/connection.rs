use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use tokio_postgres::types::ToSql;

pub struct PostgresConnection {
    pub(crate) client: deadpool_postgres::Client,
    #[cfg(feature = "tracing")]
    slow_query_threshold: Option<std::time::Duration>,
}

impl PostgresConnection {
    pub(crate) fn new(client: deadpool_postgres::Client) -> Self {
        Self {
            client,
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
impl IAsyncConnection for PostgresConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        let pgp = crate::type_conversion::db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pgp
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        self.client
            .execute(sql, &refs)
            .await
            .map_err(|e| EFError::Query(format!("Execution error: {}", e)))
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>> {
        let _guard = rust_ef::observability::QueryGuard::new(sql, self.threshold());
        let pgp = crate::type_conversion::db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pgp
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        let rows = self
            .client
            .query(sql, &refs)
            .await
            .map_err(|e| EFError::Query(format!("Query error: {}", e)))?;
        let columns: Vec<&tokio_postgres::Column> = if !rows.is_empty() {
            rows[0].columns().iter().collect()
        } else {
            Vec::new()
        };
        let result = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, col)| crate::row_conversion::cell_to_string(row, i, col.type_()))
                    .collect()
            })
            .collect();
        Ok(result)
    }

    async fn begin_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("BEGIN")
            .await
            .map_err(|e| EFError::Transaction(format!("BEGIN failed: {}", e)))?;
        Ok(())
    }

    async fn commit_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("COMMIT")
            .await
            .map_err(|e| EFError::Transaction(format!("COMMIT failed: {}", e)))?;
        Ok(())
    }

    async fn rollback_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("ROLLBACK")
            .await
            .map_err(|e| EFError::Transaction(format!("ROLLBACK failed: {}", e)))?;
        Ok(())
    }

    async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.client
            .simple_query(&format!("SAVEPOINT {}", name))
            .await
            .map_err(|e| EFError::Transaction(format!("SAVEPOINT failed: {}", e)))?;
        Ok(())
    }

    async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.client
            .simple_query(&format!("RELEASE SAVEPOINT {}", name))
            .await
            .map_err(|e| EFError::Transaction(format!("RELEASE failed: {}", e)))?;
        Ok(())
    }

    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
        self.client
            .simple_query(&format!("ROLLBACK TO SAVEPOINT {}", name))
            .await
            .map_err(|e| EFError::Transaction(format!("ROLLBACK TO failed: {}", e)))?;
        Ok(())
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
        self.client
            .simple_query(&sql)
            .await
            .map_err(|e| EFError::Transaction(format!("SET ISOLATION failed: {}", e)))?;
        Ok(())
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&mut self, threshold: std::time::Duration) {
        self.slow_query_threshold = Some(threshold);
    }
}
