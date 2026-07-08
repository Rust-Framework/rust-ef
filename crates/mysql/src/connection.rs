use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use sqlx::Row;

/// Converts a cell from a `sqlx::mysql::MySqlRow` into a `String` suitable for
/// the `IFromRow::from_row(&[String])` interface.
///
/// Dispatches by attempting `try_get` on common Rust types in a specific
/// order (bool → i64 → u64 → f64 → NaiveDateTime → NaiveDate → Uuid →
/// String → Vec<u8>). This mirrors the PostgreSQL provider's
/// `cell_to_string` approach and fixes the bug where non-String columns
/// (integers, booleans, datetimes, bytes) were silently converted to
/// `"NULL"` because `try_get::<String>` fails on non-text types.
///
/// `Ok(None)` from `try_get::<Option<T>>` indicates a SQL NULL — returned as
/// `"NULL"` to match the SQLite/PG provider convention. `Err` indicates the
/// column's native type is not representable as `T`, so we fall through to
/// the next candidate type.
fn cell_to_string(row: &sqlx::mysql::MySqlRow, col_idx: usize) -> String {
    // bool (TINYINT(1)) — must precede integer types, since MySQL booleans
    // are stored as 0/1 and would otherwise be matched by i64.
    if let Ok(v) = row.try_get::<Option<bool>, _>(col_idx) {
        return v
            .map(|b| if b { "1".into() } else { "0".into() })
            .unwrap_or_else(|| "NULL".into());
    }
    // i64 covers most signed integer columns (TINYINT, SMALLINT, INT, BIGINT)
    if let Ok(v) = row.try_get::<Option<i64>, _>(col_idx) {
        return v.map(|n| n.to_string()).unwrap_or_else(|| "NULL".into());
    }
    // u64 for unsigned big ints (BIGINT UNSIGNED)
    if let Ok(v) = row.try_get::<Option<u64>, _>(col_idx) {
        return v.map(|n| n.to_string()).unwrap_or_else(|| "NULL".into());
    }
    // f64 (FLOAT, DOUBLE)
    if let Ok(v) = row.try_get::<Option<f64>, _>(col_idx) {
        return v.map(|n| n.to_string()).unwrap_or_else(|| "NULL".into());
    }
    // NaiveDateTime (DATETIME, TIMESTAMP)
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(col_idx) {
        return v.map(|dt| dt.to_string()).unwrap_or_else(|| "NULL".into());
    }
    // NaiveDate (DATE)
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(col_idx) {
        return v.map(|d| d.to_string()).unwrap_or_else(|| "NULL".into());
    }
    // Uuid (CHAR(36))
    if let Ok(v) = row.try_get::<Option<uuid::Uuid>, _>(col_idx) {
        return v.map(|u| u.to_string()).unwrap_or_else(|| "NULL".into());
    }
    // String (VARCHAR, TEXT, CHAR) — also catches DECIMAL since MySQL
    // returns DECIMAL as string to preserve precision.
    if let Ok(v) = row.try_get::<Option<String>, _>(col_idx) {
        return v.unwrap_or_else(|| "NULL".into());
    }
    // bytes (BLOB, BINARY)
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(col_idx) {
        return v
            .map(|b| format!("{:?}", b))
            .unwrap_or_else(|| "NULL".into());
    }
    // Unknown / unsupported type — last resort
    "NULL".into()
}

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

        let result = rows
            .iter()
            .map(|row| {
                row.columns()
                    .iter()
                    .enumerate()
                    .map(|(i, _)| cell_to_string(row, i))
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
}
