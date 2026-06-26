//! Database provider abstraction trait.
//!
//! Corresponds to EFCore's database provider model, allowing multiple
//! database backends (PostgreSQL, MySQL, SQLite, etc.) to be plugged in.

use crate::error::EFResult;
use async_trait::async_trait;
use std::fmt;

/// A typed database parameter value for parameterized queries.
#[derive(Debug, Clone, PartialEq)]
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
impl From<&i32> for DbValue {
    fn from(v: &i32) -> Self {
        DbValue::I32(*v)
    }
}
impl From<i64> for DbValue {
    fn from(v: i64) -> Self {
        DbValue::I64(v)
    }
}
impl From<&i64> for DbValue {
    fn from(v: &i64) -> Self {
        DbValue::I64(*v)
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

// --- Feature-gated From impls for chrono / uuid / decimal ---

#[cfg(feature = "chrono")]
impl From<chrono::DateTime<chrono::Utc>> for DbValue {
    fn from(dt: chrono::DateTime<chrono::Utc>) -> Self {
        DbValue::String(dt.to_rfc3339())
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDateTime> for DbValue {
    fn from(ndt: chrono::NaiveDateTime) -> Self {
        DbValue::String(ndt.to_string())
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDate> for DbValue {
    fn from(nd: chrono::NaiveDate) -> Self {
        DbValue::String(nd.to_string())
    }
}

#[cfg(feature = "uuid")]
impl From<uuid::Uuid> for DbValue {
    fn from(u: uuid::Uuid) -> Self {
        DbValue::String(u.to_string())
    }
}

#[cfg(feature = "decimal")]
impl From<rust_decimal::Decimal> for DbValue {
    fn from(d: rust_decimal::Decimal) -> Self {
        DbValue::String(d.to_string())
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

/// Error returned when a [`DbValue`] cannot be converted to the requested type.
///
/// Used by `TryFrom<DbValue>` impls for `i32`/`i64`/`f64`/`String`/`bool`/...
/// and by `QueryBuilder::min_internal` / `max_internal` to surface type
/// mismatches when reading aggregation results.
#[derive(Debug, Clone, PartialEq)]
pub struct DbValueConvertError {
    /// The [`DbValue`] that could not be converted.
    pub source: DbValue,
    /// The target Rust type name (e.g. `"i32"`).
    pub target_type: &'static str,
}

impl fmt::Display for DbValueConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot convert {:?} to {}",
            self.source, self.target_type
        )
    }
}

impl std::error::Error for DbValueConvertError {}

impl From<DbValueConvertError> for crate::error::EFError {
    fn from(e: DbValueConvertError) -> Self {
        crate::error::EFError::TypeConversion(e.to_string())
    }
}

impl TryFrom<DbValue> for i32 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::I32(n) => Ok(n),
            DbValue::I16(n) => Ok(n as i32),
            DbValue::I64(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I64(n),
                target_type: "i32",
            }),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "i32",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "i32",
            }),
        }
    }
}

impl TryFrom<DbValue> for i64 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::I64(n) => Ok(n),
            DbValue::I32(n) => Ok(n as i64),
            DbValue::I16(n) => Ok(n as i64),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "i64",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "i64",
            }),
        }
    }
}

impl TryFrom<DbValue> for f64 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::F64(x) => Ok(x),
            DbValue::F32(x) => Ok(x as f64),
            DbValue::I32(n) => Ok(n as f64),
            DbValue::I64(n) => Ok(n as f64),
            DbValue::I16(n) => Ok(n as f64),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "f64",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "f64",
            }),
        }
    }
}

impl TryFrom<DbValue> for f32 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::F32(x) => Ok(x),
            DbValue::F64(x) => Ok(x as f32),
            DbValue::I32(n) => Ok(n as f32),
            DbValue::I64(n) => Ok(n as f32),
            DbValue::I16(n) => Ok(n as f32),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "f32",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "f32",
            }),
        }
    }
}

impl TryFrom<DbValue> for String {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::String(s) => Ok(s),
            DbValue::Bool(b) => Ok(b.to_string()),
            DbValue::I16(n) => Ok(n.to_string()),
            DbValue::I32(n) => Ok(n.to_string()),
            DbValue::I64(n) => Ok(n.to_string()),
            DbValue::F32(x) => Ok(x.to_string()),
            DbValue::F64(x) => Ok(x.to_string()),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "String",
            }),
        }
    }
}

impl TryFrom<DbValue> for bool {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::Bool(b) => Ok(b),
            DbValue::I64(n) => Ok(n != 0),
            DbValue::I32(n) => Ok(n != 0),
            DbValue::I16(n) => Ok(n != 0),
            DbValue::String(s) => {
                let lower = s.to_ascii_lowercase();
                match lower.as_str() {
                    "true" | "t" | "1" => Ok(true),
                    "false" | "f" | "0" => Ok(false),
                    _ => Err(DbValueConvertError {
                        source: DbValue::String(s),
                        target_type: "bool",
                    }),
                }
            }
            other => Err(DbValueConvertError {
                source: other,
                target_type: "bool",
            }),
        }
    }
}

impl TryFrom<DbValue> for Vec<u8> {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::Bytes(b) => Ok(b),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "Vec<u8>",
            }),
        }
    }
}

impl TryFrom<DbValue> for i16 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::I16(n) => Ok(n),
            DbValue::I32(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I32(n),
                target_type: "i16",
            }),
            DbValue::I64(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I64(n),
                target_type: "i16",
            }),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "i16",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "i16",
            }),
        }
    }
}

/// Represents a SQL dialect with specific syntax for common operations.
pub trait ISqlGenerator: Send + Sync {
    /// Generates a SELECT statement.
    fn select(&self, table: &str, columns: &[&str]) -> String;
    /// Generates an INSERT statement.
    fn insert(&self, table: &str, columns: &[&str], returning: bool) -> String;
    /// Generates a multi-row INSERT statement with `row_count` value groups
    /// (`INSERT INTO t (c1, c2) VALUES (?, ?), (?, ?), ...`). Placeholders
    /// follow the dialect's numbering (`?` for SQLite/MySQL, `$n` for PG).
    fn insert_batch(&self, table: &str, columns: &[&str], row_count: usize) -> String {
        let _ = (table, columns, row_count);
        String::new()
    }
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
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64>;
    /// Executes a query with parameters and returns rows.
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>>;
    /// Begins a transaction.
    async fn begin_transaction(&mut self) -> EFResult<()>;
    /// Commits the current transaction.
    async fn commit_transaction(&mut self) -> EFResult<()>;
    /// Rolls back the current transaction.
    async fn rollback_transaction(&mut self) -> EFResult<()>;
}

/// The database provider abstraction.
/// Corresponds to EFCore's provider model.
#[async_trait]
pub trait IDatabaseProvider: Send + Sync {
    /// Returns the SQL dialect generator for this provider.
    ///
    /// Implementations are stateless, so a `&'static` reference is returned —
    /// no heap allocation per call.
    fn sql_generator(&self) -> &'static dyn ISqlGenerator;

    /// Gets an async database connection from the pool.
    async fn get_connection(&self) -> EFResult<Box<dyn IAsyncConnection>>;

    /// Executes a migration command (DDL).
    async fn execute_migration_command(&self, sql: &str) -> EFResult<()>;

    /// Returns the provider name (e.g., "PostgreSQL", "MySQL").
    fn name(&self) -> &str;

    /// Returns the migration dialect for this provider.
    fn migration_dialect(&self) -> crate::migration::MigrationDialect;
}
