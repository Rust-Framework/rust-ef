//! MySQL provider for Rust Entity Framework.
//!
//! Implements `IDatabaseProvider`, `ISqlGenerator`, and `IAsyncConnection`
//! traits for MySQL via `sqlx` with async connection pooling.
//!
//! Also provides `DbContextOptionsBuilderExt` for EFCore-style configuration:
//! `.use_mysql("mysql://user:pass@localhost/db")`

use async_trait::async_trait;
use lref::error::{LrefError, LrefResult};
use lref::provider::{DbValue, IAsyncConnection, IDatabaseProvider, ISqlGenerator};
use sqlx::{Column, Row};

// ---------------------------------------------------------------------------
// MySQL SQL Generator
// ---------------------------------------------------------------------------

/// MySQL-specific SQL dialect generator.
#[derive(Debug, Clone)]
pub struct MySqlSqlGenerator;

impl MySqlSqlGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl ISqlGenerator for MySqlSqlGenerator {
    fn select(&self, table: &str, columns: &[&str]) -> String {
        let cols = if columns.is_empty() {
            "*".to_string()
        } else {
            columns
                .iter()
                .map(|c| self.quote_identifier(c))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("SELECT {} FROM {}", cols, self.quote_identifier(table))
    }

    fn insert(&self, table: &str, columns: &[&str], _returning: bool) -> String {
        let cols = columns
            .iter()
            .map(|c| self.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = vec!["?"; columns.len()].join(", ");
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.quote_identifier(table),
            cols,
            placeholders
        )
    }

    fn update(&self, table: &str, set_columns: &[&str], where_clause: &str) -> String {
        let sets: Vec<String> = set_columns
            .iter()
            .map(|c| format!("{} = ?", self.quote_identifier(c)))
            .collect();
        format!(
            "UPDATE {} SET {} {}",
            self.quote_identifier(table),
            sets.join(", "),
            where_clause
        )
    }

    fn delete(&self, table: &str, where_clause: &str) -> String {
        format!(
            "DELETE FROM {} {}",
            self.quote_identifier(table),
            where_clause
        )
    }

    fn create_table(&self, table: &str, columns: &[(String, String)]) -> String {
        let col_defs: Vec<String> = columns
            .iter()
            .map(|(name, type_def)| format!("{} {}", self.quote_identifier(name), type_def))
            .collect();
        format!(
            "CREATE TABLE {} (\n    {}\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4",
            self.quote_identifier(table),
            col_defs.join(",\n    ")
        )
    }

    fn drop_table(&self, table: &str) -> String {
        format!("DROP TABLE IF EXISTS {}", self.quote_identifier(table))
    }

    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String {
        match (skip, take) {
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            (Some(s), None) => format!("LIMIT 18446744073709551615 OFFSET {}", s),
            (None, None) => String::new(),
        }
    }

    fn parameter_placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("`{}`", identifier)
    }

    fn auto_increment_syntax(&self) -> &'static str {
        "AUTO_INCREMENT"
    }
}

// ---------------------------------------------------------------------------
// MySQL Provider
// ---------------------------------------------------------------------------

/// MySQL database provider with connection pooling via sqlx.
pub struct MySqlProvider {
    pool: sqlx::MySqlPool,
}

impl MySqlProvider {
    /// Creates a new MySQL provider with a connection pool.
    pub async fn new(connection_string: &str) -> LrefResult<Self> {
        let pool = sqlx::MySqlPool::connect(connection_string)
            .await
            .map_err(|e| LrefError::Connection(format!("MySQL connection failed: {}", e)))?;
        Ok(Self { pool })
    }

    /// Creates a provider with an existing pool.
    pub fn from_pool(pool: sqlx::MySqlPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IDatabaseProvider for MySqlProvider {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator> {
        Box::new(MySqlSqlGenerator::new())
    }

    async fn get_connection(&self) -> LrefResult<Box<dyn IAsyncConnection>> {
        let conn = self
            .pool
            .acquire()
            .await
            .map_err(|e| LrefError::Connection(format!("Pool acquire failed: {}", e)))?;

        Ok(Box::new(MySqlConnection::new(conn)))
    }

    async fn execute_migration_command(&self, sql: &str) -> LrefResult<()> {
        sqlx::query(sql)
            .execute(&self.pool)
            .await
            .map_err(|e| LrefError::Migration(format!("Migration execution failed: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "MySQL"
    }
}

impl std::fmt::Debug for MySqlProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MySqlProvider")
            .field("name", &self.name())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// MySQL Connection
// ---------------------------------------------------------------------------

/// Async connection wrapper for MySQL.
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
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<u64> {
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| LrefError::Connection("Connection already closed".to_string()))?;
        let result = build_mysql_query(sql, params)
            .execute(&mut **conn)
            .await
            .map_err(|e| LrefError::Query(format!("Execution error: {}", e)))?;
        Ok(result.rows_affected())
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<Vec<Vec<String>>> {
        let conn = self
            .conn
            .as_mut()
            .ok_or_else(|| LrefError::Connection("Connection already closed".to_string()))?;
        let rows = build_mysql_query(sql, params)
            .fetch_all(&mut **conn)
            .await
            .map_err(|e| LrefError::Query(format!("Query error: {}", e)))?;

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

    async fn begin_transaction(&mut self) -> LrefResult<()> {
        self.execute("START TRANSACTION", &[]).await.map(|_| ())
    }

    async fn commit_transaction(&mut self) -> LrefResult<()> {
        self.execute("COMMIT", &[]).await.map(|_| ())
    }

    async fn rollback_transaction(&mut self) -> LrefResult<()> {
        self.execute("ROLLBACK", &[]).await.map(|_| ())
    }
}

fn build_mysql_query<'q>(
    sql: &'q str,
    params: &'q [DbValue],
) -> sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments> {
    let mut query = sqlx::query::<sqlx::MySql>(sql);
    for param in params {
        query = match param {
            DbValue::Null => query.bind(None::<String>),
            DbValue::Bool(v) => query.bind(*v),
            DbValue::I16(v) => query.bind(*v),
            DbValue::I32(v) => query.bind(*v),
            DbValue::I64(v) => query.bind(*v),
            DbValue::F32(v) => query.bind(*v),
            DbValue::F64(v) => query.bind(*v),
            DbValue::String(v) => query.bind(v.as_str()),
            DbValue::Bytes(v) => query.bind(v.as_slice()),
        };
    }
    query
}

// ---------------------------------------------------------------------------
// DbContextOptionsBuilder extension -- EFCore-style .UseMySql()
// ---------------------------------------------------------------------------

/// Extension trait that adds `.use_mysql()` to `DbContextOptionsBuilder`.
pub trait DbContextOptionsBuilderExt {
    /// Configures the context to use MySQL.
    fn use_mysql(&mut self, connection_string: &str) -> &mut Self;
}

impl DbContextOptionsBuilderExt for lref::db_context::DbContextOptionsBuilder {
    fn use_mysql(&mut self, connection_string: &str) -> &mut Self {
        self.set_provider("mysql", connection_string)
    }
}
