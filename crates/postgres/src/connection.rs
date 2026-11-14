use async_trait::async_trait;
use rust_ef::error::EFResult;
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use tokio_postgres::types::ToSql;

use crate::error_map::map_pg_error;

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
            .map_err(|e| map_pg_error(e, "Execution error", sql))
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<DbValue>>> {
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
            .map_err(|e| map_pg_error(e, "Query error", sql))?;
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
                    .map(|(i, col)| crate::row_conversion::cell_to_db_value(row, i, col.type_()))
                    .collect()
            })
            .collect();
        Ok(result)
    }

    async fn begin_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("BEGIN")
            .await
            .map_err(|e| map_pg_error(e, "BEGIN failed", "BEGIN"))?;
        Ok(())
    }

    async fn commit_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("COMMIT")
            .await
            .map_err(|e| map_pg_error(e, "COMMIT failed", "COMMIT"))?;
        Ok(())
    }

    async fn rollback_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("ROLLBACK")
            .await
            .map_err(|e| map_pg_error(e, "ROLLBACK failed", "ROLLBACK"))?;
        Ok(())
    }

    async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
        let sql = format!("SAVEPOINT {}", name);
        self.client
            .simple_query(&sql)
            .await
            .map_err(|e| map_pg_error(e, "SAVEPOINT failed", &sql))?;
        Ok(())
    }

    async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
        let sql = format!("RELEASE SAVEPOINT {}", name);
        self.client
            .simple_query(&sql)
            .await
            .map_err(|e| map_pg_error(e, "RELEASE failed", &sql))?;
        Ok(())
    }

    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
        let sql = format!("ROLLBACK TO SAVEPOINT {}", name);
        self.client
            .simple_query(&sql)
            .await
            .map_err(|e| map_pg_error(e, "ROLLBACK TO failed", &sql))?;
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
            .map_err(|e| map_pg_error(e, "SET ISOLATION failed", &sql))?;
        Ok(())
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&mut self, threshold: std::time::Duration) {
        self.slow_query_threshold = Some(threshold);
    }

    fn discard(self: Box<Self>) {
        let this = *self;
        // Take the connection out of the deadpool object; dropping it closes
        // the connection, and PostgreSQL rolls back on disconnect.
        let _ = deadpool_postgres::Client::take(this.client);
    }
}

#[cfg(test)]
mod tests {
    use tokio_postgres::error::SqlState;

    #[test]
    fn unique_violation_sqlstate_is_23505() {
        assert_eq!(SqlState::UNIQUE_VIOLATION.code(), "23505");
    }
}
