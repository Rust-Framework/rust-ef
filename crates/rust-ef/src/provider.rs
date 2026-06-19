//! Database provider abstraction trait.
//!
//! Corresponds to EFCore's database provider model, allowing multiple
//! database backends (PostgreSQL, MySQL, SQLite, etc.) to be plugged in.

use crate::error::LrefResult;
use async_trait::async_trait;
use std::fmt;

/// A typed database parameter value for parameterized queries.
#[derive(Debug, Clone)]
pub enum DbValue {
    Null,
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
}

impl fmt::Display for DbValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbValue::Null => write!(f, "NULL"),
            DbValue::Bool(v) => write!(f, "{}", if *v { "TRUE" } else { "FALSE" }),
            DbValue::I16(v) => write!(f, "{}", v),
            DbValue::I32(v) => write!(f, "{}", v),
            DbValue::I64(v) => write!(f, "{}", v),
            DbValue::F32(v) => write!(f, "{}", v),
            DbValue::F64(v) => write!(f, "{}", v),
            DbValue::String(v) => write!(f, "'{}'", v.replace('\'', "''")),
            DbValue::Bytes(v) => write!(f, "{}", hex::encode(v)),
        }
    }
}

mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

impl From<i32> for DbValue {
    fn from(v: i32) -> Self {
        DbValue::I32(v)
    }
}
impl From<i64> for DbValue {
    fn from(v: i64) -> Self {
        DbValue::I64(v)
    }
}
impl From<String> for DbValue {
    fn from(v: String) -> Self {
        DbValue::String(v)
    }
}
impl From<&str> for DbValue {
    fn from(v: &str) -> Self {
        DbValue::String(v.to_string())
    }
}
impl From<bool> for DbValue {
    fn from(v: bool) -> Self {
        DbValue::Bool(v)
    }
}
impl From<f64> for DbValue {
    fn from(v: f64) -> Self {
        DbValue::F64(v)
    }
}
impl From<f32> for DbValue {
    fn from(v: f32) -> Self {
        DbValue::F32(v)
    }
}
impl From<i16> for DbValue {
    fn from(v: i16) -> Self {
        DbValue::I16(v)
    }
}
impl From<Vec<u8>> for DbValue {
    fn from(v: Vec<u8>) -> Self {
        DbValue::Bytes(v)
    }
}
impl<T> From<Option<T>> for DbValue
where
    T: Into<DbValue>,
{
    fn from(v: Option<T>) -> Self {
        match v {
            Some(val) => val.into(),
            None => DbValue::Null,
        }
    }
}

/// Represents a SQL dialect with specific syntax for common operations.
pub trait ISqlGenerator: Send + Sync {
    /// Generates a SELECT statement.
    fn select(&self, table: &str, columns: &[&str]) -> String;
    /// Generates an INSERT statement.
    fn insert(&self, table: &str, columns: &[&str], returning: bool) -> String;
    /// Generates an UPDATE statement.
    fn update(&self, table: &str, set_columns: &[&str], where_clause: &str) -> String;
    /// Generates a DELETE statement.
    fn delete(&self, table: &str, where_clause: &str) -> String;
    /// Generates a CREATE TABLE statement.
    fn create_table(&self, table: &str, columns: &[(String, String)]) -> String;
    /// Generates a DROP TABLE statement.
    fn drop_table(&self, table: &str) -> String;
    /// Generates a pagination clause.
    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String;
    /// Returns the parameter placeholder (e.g., `$1` for PG, `?` for MySQL).
    fn parameter_placeholder(&self, index: usize) -> String;
    /// Returns the identifier quoting character (e.g., `"` for PG, `` ` `` for MySQL).
    fn quote_identifier(&self, identifier: &str) -> String;
    /// Returns the dialect-specific auto-increment syntax.
    fn auto_increment_syntax(&self) -> &'static str;
}

/// Trait for async database connections.
#[async_trait]
pub trait IAsyncConnection: Send + Sync {
    /// Executes a query with parameters and returns the number of affected rows.
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<u64>;
    /// Executes a query with parameters and returns rows.
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<Vec<Vec<String>>>;
    /// Begins a transaction.
    async fn begin_transaction(&mut self) -> LrefResult<()>;
    /// Commits the current transaction.
    async fn commit_transaction(&mut self) -> LrefResult<()>;
    /// Rolls back the current transaction.
    async fn rollback_transaction(&mut self) -> LrefResult<()>;
}

/// The database provider abstraction.
/// Corresponds to EFCore's provider model.
#[async_trait]
pub trait IDatabaseProvider: Send + Sync {
    /// Returns the SQL dialect generator for this provider.
    fn sql_generator(&self) -> Box<dyn ISqlGenerator>;

    /// Gets an async database connection from the pool.
    async fn get_connection(&self) -> LrefResult<Box<dyn IAsyncConnection>>;

    /// Executes a migration command (DDL).
    async fn execute_migration_command(&self, sql: &str) -> LrefResult<()>;

    /// Returns the provider name (e.g., "PostgreSQL", "MySQL").
    fn name(&self) -> &str;
}
