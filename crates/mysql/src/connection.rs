use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection};
use sqlx::{Column, Row};

pub struct MySqlConnection {
    conn: Option<sqlx::pool::PoolConnection<sqlx::MySql>>,
}

impl MySqlConnection {
    pub(crate) fn new(conn: sqlx::pool::PoolConnection<sqlx::MySql>) -> Self {
        Self { conn: Some(conn) }
    }
}

#[async_trait]
impl IAsyncConnection for MySqlConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
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

        let columns: Vec<String> = rows[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        let result = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| {
                        row.try_get::<String, _>(i)
                            .unwrap_or_else(|_| "NULL".to_string())
                    })
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
}
