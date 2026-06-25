use async_trait::async_trait;
use rust_ef::error::{EfError, EfResult};
use rust_ef::provider::{DbValue, IAsyncConnection};
use tokio_postgres::types::ToSql;

pub struct PostgresConnection {
    pub(crate) client: deadpool_postgres::Client,
}

#[async_trait]
impl IAsyncConnection for PostgresConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EfResult<u64> {
        let pgp = crate::type_conversion::db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pgp
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        self.client
            .execute(sql, &refs)
            .await
            .map_err(|e| EfError::Query(format!("Execution error: {}", e)))
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EfResult<Vec<Vec<String>>> {
        let pgp = crate::type_conversion::db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pgp
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        let rows = self
            .client
            .query(sql, &refs)
            .await
            .map_err(|e| EfError::Query(format!("Query error: {}", e)))?;
        let columns: Vec<String> = if !rows.is_empty() {
            rows[0]
                .columns()
                .iter()
                .map(|c| c.name().to_string())
                .collect()
        } else {
            Vec::new()
        };
        let result = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        row.try_get::<_, String>(i)
                            .unwrap_or_else(|_| "NULL".to_string())
                    })
                    .collect()
            })
            .collect();
        Ok(result)
    }

    async fn begin_transaction(&mut self) -> EfResult<()> {
        self.client
            .simple_query("BEGIN")
            .await
            .map_err(|e| EfError::Transaction(format!("BEGIN failed: {}", e)))?;
        Ok(())
    }

    async fn commit_transaction(&mut self) -> EfResult<()> {
        self.client
            .simple_query("COMMIT")
            .await
            .map_err(|e| EfError::Transaction(format!("COMMIT failed: {}", e)))?;
        Ok(())
    }

    async fn rollback_transaction(&mut self) -> EfResult<()> {
        self.client
            .simple_query("ROLLBACK")
            .await
            .map_err(|e| EfError::Transaction(format!("ROLLBACK failed: {}", e)))?;
        Ok(())
    }
}
