//! SQLite provider for Rust Entity Framework.

use async_trait::async_trait;
use rust_ef::error::{LrefError, LrefResult};
use rust_ef::provider::{DbValue, IAsyncConnection, IDatabaseProvider, ISqlGenerator};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// SQLite SQL Generator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SqliteSqlGenerator;

impl SqliteSqlGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SqliteSqlGenerator {
    fn default() -> Self {
        Self::new()
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

pub struct SqliteProvider {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl SqliteProvider {
    pub fn new(path: impl AsRef<Path>) -> LrefResult<Self> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| LrefError::Connection(format!("SQLite open failed: {}", e)))?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| LrefError::Connection(format!("SQLite WAL setup failed: {}", e)))?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }
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
        let rp = to_rusqlite_params(params);
        let refs: Vec<&dyn rusqlite::types::ToSql> = rp
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        conn.execute(sql, refs.as_slice())
            .map(|c| c as u64)
            .map_err(|e| LrefError::Query(format!("Execution error: {}", e)))
    }
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<Vec<Vec<String>>> {
        let conn = self.conn.lock().await;
        let rp = to_rusqlite_params(params);
        let refs: Vec<&dyn rusqlite::types::ToSql> = rp
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| LrefError::Query(format!("Prepare error: {}", e)))?;
        let cc = stmt.column_count();
        let rows = stmt
            .query_map(refs.as_slice(), |row| {
                let mut vals = Vec::with_capacity(cc);
                for i in 0..cc {
                    vals.push(
                        row.get::<_, String>(i)
                            .or_else(|_| row.get::<_, i64>(i).map(|n| n.to_string()))
                            .or_else(|_| row.get::<_, f64>(i).map(|n| n.to_string()))
                            .unwrap_or_else(|_| "NULL".to_string()),
                    );
                }
                Ok(vals)
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

// ---------------------------------------------------------------------------
// DbContextOptionsBuilder extension — .use_sqlite()
// ---------------------------------------------------------------------------

pub trait DbContextOptionsBuilderExt {
    fn use_sqlite(&mut self, connection_string: &str) -> &mut Self;
    fn use_sqlite_in_memory(&mut self) -> &mut Self;
}

impl DbContextOptionsBuilderExt for rust_ef::db_context::DbContextOptionsBuilder {
    fn use_sqlite(&mut self, connection_string: &str) -> &mut Self {
        let cs = connection_string.to_string();
        self.set_provider_factory(
            "sqlite",
            &cs,
            Arc::new(move |cs: &str| {
                Ok(Arc::new(SqliteProvider::new(cs)?) as Arc<dyn IDatabaseProvider>)
            }),
        )
    }
    fn use_sqlite_in_memory(&mut self) -> &mut Self {
        self.set_provider_factory(
            "sqlite",
            ":memory:",
            Arc::new(|_cs: &str| {
                Ok(Arc::new(SqliteProvider::new_in_memory()?) as Arc<dyn IDatabaseProvider>)
            }),
        )
    }
}
