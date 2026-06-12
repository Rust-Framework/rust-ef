//! SQLite provider for Rust Entity Framework.
//!
//! Implements `IDatabaseProvider`, `ISqlGenerator`, and `IAsyncConnection`
//! traits for SQLite via `rusqlite` with a tokio-compatible async wrapper.

use async_trait::async_trait;
use lref::error::{LrefError, LrefResult};
use lref::provider::{DbValue, IAsyncConnection, IDatabaseProvider, ISqlGenerator};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// SQLite SQL Generator
// ---------------------------------------------------------------------------

/// SQLite-specific SQL dialect generator.
#[derive(Debug, Clone)]
pub struct SqliteSqlGenerator;

impl SqliteSqlGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl ISqlGenerator for SqliteSqlGenerator {
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
            "CREATE TABLE {} (\n    {}\n)",
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
            _ => String::new(),
        }
    }

    fn parameter_placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }

    fn auto_increment_syntax(&self) -> &'static str {
        "AUTOINCREMENT"
    }
}

// ---------------------------------------------------------------------------
// SQLite Provider
// ---------------------------------------------------------------------------

/// SQLite database provider with an async-compatible connection wrapper.
pub struct SqliteProvider {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteProvider {
    /// Creates a new SQLite provider connected to a file.
    pub fn new(path: impl AsRef<Path>) -> LrefResult<Self> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| LrefError::Connection(format!("SQLite open failed: {}", e)))?;

        // Enable WAL mode for better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| LrefError::Connection(format!("SQLite WAL setup failed: {}", e)))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Creates an in-memory SQLite provider.
    pub fn new_in_memory() -> LrefResult<Self> {
        Self::new(":memory:")
    }
}

#[async_trait]
impl IDatabaseProvider for SqliteProvider {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator> {
        Box::new(SqliteSqlGenerator::new())
    }

    async fn get_connection(&self) -> LrefResult<Box<dyn IAsyncConnection>> {
        Ok(Box::new(SqliteConnection::new(self.conn.clone())))
    }

    async fn execute_migration_command(&self, sql: &str) -> LrefResult<()> {
        let conn = self.conn.lock().await;
        conn.execute_batch(sql)
            .map_err(|e| LrefError::Migration(format!("Migration execution failed: {}", e)))?;
        Ok(())
    }

    fn name(&self) -> &str {
        "SQLite"
    }
}

impl std::fmt::Debug for SqliteProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteProvider")
            .field("name", &self.name())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// SQLite Connection
// ---------------------------------------------------------------------------

/// Async-compatible connection wrapper for SQLite.
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
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<u64> {
        let conn = self.conn.lock().await;
        let rusqlite_params = to_rusqlite_params(params);
        let refs: Vec<&dyn rusqlite::types::ToSql> = rusqlite_params
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        let changes = conn
            .execute(sql, refs.as_slice())
            .map_err(|e| LrefError::Query(format!("Execution error: {}", e)))?;
        Ok(changes as u64)
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<Vec<Vec<String>>> {
        let conn = self.conn.lock().await;
        let rusqlite_params = to_rusqlite_params(params);
        let refs: Vec<&dyn rusqlite::types::ToSql> = rusqlite_params
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| LrefError::Query(format!("Prepare error: {}", e)))?;

        let column_count = stmt.column_count();

        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                let mut values = Vec::with_capacity(column_count);
                for i in 0..column_count {
                    // Try String first, then i64, then f64, then fallback to NULL
                    let val = row
                        .get::<_, String>(i)
                        .or_else(|_| row.get::<_, i64>(i).map(|n| n.to_string()))
                        .or_else(|_| row.get::<_, f64>(i).map(|n| n.to_string()))
                        .unwrap_or_else(|_| "NULL".to_string());
                    values.push(val);
                }
                Ok(values)
            })
            .map_err(|e| LrefError::Query(format!("Query error: {}", e)))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| LrefError::Query(format!("Row read error: {}", e)))?);
        }

        Ok(result)
    }

    async fn begin_transaction(&mut self) -> LrefResult<()> {
        self.execute("BEGIN TRANSACTION", &[]).await.map(|_| ())
    }

    async fn commit_transaction(&mut self) -> LrefResult<()> {
        self.execute("COMMIT", &[]).await.map(|_| ())
    }

    async fn rollback_transaction(&mut self) -> LrefResult<()> {
        self.execute("ROLLBACK", &[]).await.map(|_| ())
    }
}

fn to_rusqlite_params(params: &[DbValue]) -> Vec<Box<dyn rusqlite::types::ToSql>> {
    params
        .iter()
        .map(|v| match v {
            DbValue::Null => Box::new(None::<String>) as Box<dyn rusqlite::types::ToSql>,
            DbValue::Bool(b) => Box::new(*b),
            DbValue::I16(n) => Box::new(*n),
            DbValue::I32(n) => Box::new(*n),
            DbValue::I64(n) => Box::new(*n),
            DbValue::F32(n) => Box::new(*n as f64),
            DbValue::F64(n) => Box::new(*n),
            DbValue::String(s) => Box::new(s.clone()),
            DbValue::Bytes(b) => Box::new(b.clone()),
        })
        .collect()
}
