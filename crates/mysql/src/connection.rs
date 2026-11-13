use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IsolationLevel};
use sqlx_core::error::ErrorKind;
use sqlx_core::executor::Executor;
use sqlx_core::raw_sql::raw_sql;
use sqlx_core::row::Row;
use sqlx_mysql::MySql;

/// Maps a `sqlx` error to a typed [`EFError`].
///
/// Duplicate-key errors (`ER_DUP_ENTRY` / `ER_DUP_KEY` / `ER_DUP_UNIQUE`) are
/// promoted to [`EFError::UniqueViolation`]; `sqlx` normalizes them to
/// [`ErrorKind::UniqueViolation`]. The table and index name are parsed from the
/// driver message (`Duplicate entry '…' for key 'table.index'`), which is the
/// only place MySQL exposes them through `sqlx`.
fn map_mysql_error(e: sqlx_core::Error, context: &str) -> EFError {
    if let sqlx_core::Error::Database(db) = &e {
        if db.kind() == ErrorKind::UniqueViolation {
            let (table, constraint) = parse_duplicate_key(db.message());
            return EFError::UniqueViolation { constraint, table };
        }
    }
    EFError::query(format!("{context}: {e}"))
}

/// Parses `(table, constraint)` from a MySQL duplicate-entry message.
///
/// `"Duplicate entry '1' for key 'order_record.ux_order_tenant'"` yields
/// `(Some("order_record"), Some("ux_order_tenant"))`. `ER_DUP_ENTRY` reports
/// only the key name in some versions, in which case the table is `None`.
fn parse_duplicate_key(message: &str) -> (Option<String>, Option<String>) {
    const MARKER: &str = "for key '";
    let Some(start) = message.find(MARKER) else {
        return (None, None);
    };
    let rest = &message[start + MARKER.len()..];
    let Some(end) = rest.find('\'') else {
        return (None, None);
    };
    let key = &rest[..end];
    match key.split_once('.') {
        Some((table, constraint)) if !table.is_empty() && !constraint.is_empty() => {
            (Some(table.to_string()), Some(constraint.to_string()))
        }
        _ => (None, Some(key.to_string())),
    }
}

pub struct MySqlConnection {
    conn: Option<sqlx_core::pool::PoolConnection<MySql>>,
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
    pub(crate) fn new(conn: sqlx_core::pool::PoolConnection<MySql>) -> Self {
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
            .execute(raw_sql(sqlx_core::sql_str::AssertSqlSafe(sql)))
            .await
            .map_err(|e| map_mysql_error(e, "Execution error"))?;
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
            .map_err(|e| map_mysql_error(e, "Execution error"))?;
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
            .map_err(|e| map_mysql_error(e, "Query error"))?;

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

#[cfg(test)]
mod tests {
    use super::parse_duplicate_key;

    #[test]
    fn parses_table_and_index_from_duplicate_entry() {
        let (table, constraint) = parse_duplicate_key(
            "Duplicate entry '1-acme' for key 'order_record.ux_order_tenant_request'",
        );
        assert_eq!(table.as_deref(), Some("order_record"));
        assert_eq!(constraint.as_deref(), Some("ux_order_tenant_request"));
    }

    #[test]
    fn parses_primary_key_duplicate_without_table() {
        let (table, constraint) = parse_duplicate_key("Duplicate entry '42' for key 'PRIMARY'");
        assert_eq!(table, None);
        assert_eq!(constraint.as_deref(), Some("PRIMARY"));
    }

    #[test]
    fn unrelated_message_yields_none() {
        assert_eq!(
            parse_duplicate_key("Table 'db.missing' doesn't exist"),
            (None, None)
        );
    }
}
