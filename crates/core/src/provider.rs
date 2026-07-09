//! Database provider abstraction trait.
//!
//! Corresponds to EFCore's database provider model, allowing multiple
//! database backends (PostgreSQL, MySQL, SQLite, etc.) to be plugged in.

use crate::error::EFResult;
use async_trait::async_trait;
use std::fmt;

/// A typed database parameter value for parameterized queries.
///
/// Native variants (`DateTime`/`NaiveDateTime`/`NaiveDate`/`Uuid`/`Decimal`)
/// are enabled by the `chrono`/`uuid`/`decimal` Cargo features. When enabled,
/// the PostgreSQL provider binds these via `tokio_postgres`'s binary protocol
/// (`with-chrono-0_4`/`with-uuid-1` features) for type-safe, lossless
/// parameter transmission. SQLite and MySQL providers collapse native
/// variants to their canonical string representation (matching v1.0 behavior)
/// since neither driver requires native type binding.
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
    /// UTC timestamp — bound natively as `TIMESTAMPTZ` on PostgreSQL.
    #[cfg(feature = "chrono")]
    DateTime(chrono::DateTime<chrono::Utc>),
    /// Naive (timezone-less) timestamp — bound natively as `TIMESTAMP` on PG.
    #[cfg(feature = "chrono")]
    NaiveDateTime(chrono::NaiveDateTime),
    /// Calendar date — bound natively as `DATE` on PostgreSQL.
    #[cfg(feature = "chrono")]
    NaiveDate(chrono::NaiveDate),
    /// UUID — bound natively as `UUID` on PostgreSQL.
    #[cfg(feature = "uuid")]
    Uuid(uuid::Uuid),
    /// Fixed-precision decimal — bound as `NUMERIC` string on PostgreSQL
    /// (tokio_postgres lacks a native `rust_decimal` adapter; the string
    /// form round-trips losslessly through PG's `NUMERIC` type).
    #[cfg(feature = "decimal")]
    Decimal(rust_decimal::Decimal),
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
            #[cfg(feature = "chrono")]
            DbValue::DateTime(v) => write!(f, "'{}'", v.to_rfc3339()),
            #[cfg(feature = "chrono")]
            DbValue::NaiveDateTime(v) => write!(f, "'{}'", v),
            #[cfg(feature = "chrono")]
            DbValue::NaiveDate(v) => write!(f, "'{}'", v),
            #[cfg(feature = "uuid")]
            DbValue::Uuid(v) => write!(f, "'{}'", v),
            #[cfg(feature = "decimal")]
            DbValue::Decimal(v) => write!(f, "'{}'", v),
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
//
// These construct native `DbValue` variants (not `String`) so that the
// PostgreSQL provider can bind them via tokio_postgres's binary protocol.
// SQLite and MySQL providers collapse these variants to their canonical
// string form in their respective `type_conversion.rs`.

#[cfg(feature = "chrono")]
impl From<chrono::DateTime<chrono::Utc>> for DbValue {
    fn from(dt: chrono::DateTime<chrono::Utc>) -> Self {
        DbValue::DateTime(dt)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDateTime> for DbValue {
    fn from(ndt: chrono::NaiveDateTime) -> Self {
        DbValue::NaiveDateTime(ndt)
    }
}

#[cfg(feature = "chrono")]
impl From<chrono::NaiveDate> for DbValue {
    fn from(nd: chrono::NaiveDate) -> Self {
        DbValue::NaiveDate(nd)
    }
}

#[cfg(feature = "uuid")]
impl From<uuid::Uuid> for DbValue {
    fn from(u: uuid::Uuid) -> Self {
        DbValue::Uuid(u)
    }
}

#[cfg(feature = "decimal")]
impl From<rust_decimal::Decimal> for DbValue {
    fn from(d: rust_decimal::Decimal) -> Self {
        DbValue::Decimal(d)
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
        crate::error::EFError::type_conversion(e.to_string())
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
            #[cfg(feature = "chrono")]
            DbValue::DateTime(dt) => Ok(dt.to_rfc3339()),
            #[cfg(feature = "chrono")]
            DbValue::NaiveDateTime(ndt) => Ok(ndt.to_string()),
            #[cfg(feature = "chrono")]
            DbValue::NaiveDate(nd) => Ok(nd.to_string()),
            #[cfg(feature = "uuid")]
            DbValue::Uuid(u) => Ok(u.to_string()),
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => Ok(d.to_string()),
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

impl TryFrom<DbValue> for i8 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::I16(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I16(n),
                target_type: "i8",
            }),
            DbValue::I32(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I32(n),
                target_type: "i8",
            }),
            DbValue::I64(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I64(n),
                target_type: "i8",
            }),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "i8",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "i8",
            }),
        }
    }
}

impl TryFrom<DbValue> for u32 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::I16(n) => (n as i32).try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I16(n),
                target_type: "u32",
            }),
            DbValue::I32(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I32(n),
                target_type: "u32",
            }),
            DbValue::I64(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I64(n),
                target_type: "u32",
            }),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "u32",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "u32",
            }),
        }
    }
}

impl TryFrom<DbValue> for u64 {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::I16(n) => (n as i64).try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I16(n),
                target_type: "u64",
            }),
            DbValue::I32(n) => (n as i64).try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I32(n),
                target_type: "u64",
            }),
            DbValue::I64(n) => n.try_into().map_err(|_| DbValueConvertError {
                source: DbValue::I64(n),
                target_type: "u64",
            }),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "u64",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "u64",
            }),
        }
    }
}

// --- Feature-gated TryFrom impls for native chrono / uuid / decimal types ---

#[cfg(feature = "chrono")]
impl TryFrom<DbValue> for chrono::DateTime<chrono::Utc> {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::DateTime(dt) => Ok(dt),
            DbValue::String(s) => chrono::DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| DbValueConvertError {
                    source: DbValue::String(s),
                    target_type: "DateTime<Utc>",
                }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "DateTime<Utc>",
            }),
        }
    }
}

#[cfg(feature = "chrono")]
impl TryFrom<DbValue> for chrono::NaiveDateTime {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::NaiveDateTime(ndt) => Ok(ndt),
            DbValue::String(s) => {
                chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                    .or_else(|_| {
                        // Fallback: ISO 8601 / RFC 3339 without timezone
                        chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S")
                    })
                    .map_err(|_| DbValueConvertError {
                        source: DbValue::String(s),
                        target_type: "NaiveDateTime",
                    })
            }
            other => Err(DbValueConvertError {
                source: other,
                target_type: "NaiveDateTime",
            }),
        }
    }
}

#[cfg(feature = "chrono")]
impl TryFrom<DbValue> for chrono::NaiveDate {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::NaiveDate(nd) => Ok(nd),
            DbValue::String(s) => {
                chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(|_| DbValueConvertError {
                    source: DbValue::String(s),
                    target_type: "NaiveDate",
                })
            }
            other => Err(DbValueConvertError {
                source: other,
                target_type: "NaiveDate",
            }),
        }
    }
}

#[cfg(feature = "uuid")]
impl TryFrom<DbValue> for uuid::Uuid {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::Uuid(u) => Ok(u),
            DbValue::String(s) => uuid::Uuid::parse_str(&s).map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "Uuid",
            }),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "Uuid",
            }),
        }
    }
}

#[cfg(feature = "decimal")]
impl TryFrom<DbValue> for rust_decimal::Decimal {
    type Error = DbValueConvertError;
    fn try_from(v: DbValue) -> Result<Self, Self::Error> {
        match v {
            DbValue::Decimal(d) => Ok(d),
            DbValue::String(s) => s.parse().map_err(|_| DbValueConvertError {
                source: DbValue::String(s),
                target_type: "Decimal",
            }),
            DbValue::I32(n) => Ok(rust_decimal::Decimal::from(n)),
            DbValue::I64(n) => Ok(rust_decimal::Decimal::from(n)),
            other => Err(DbValueConvertError {
                source: other,
                target_type: "Decimal",
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
    /// Generates a batch UPDATE using `CASE pk_col WHEN ? THEN ?` for
    /// `row_count` rows, reducing N round trips to 1.
    ///
    /// The SET clause uses `2 * set_columns.len() * row_count` placeholders
    /// (numbered from 1). The caller-built `where_clause` must number its
    /// placeholders starting from `2 * set_columns.len() * row_count + 1`.
    ///
    /// Parameter layout (caller must arrange params in this order):
    /// - For each set column, for each row: `[pk_value, col_value]`
    /// - Then the `where_clause` params (PK IN-list + optional filter)
    fn update_batch(
        &self,
        table: &str,
        set_columns: &[&str],
        pk_col: &str,
        row_count: usize,
        where_clause: &str,
    ) -> String {
        let mut idx = 1usize;
        let sets: Vec<String> = set_columns
            .iter()
            .map(|col| {
                let whens: Vec<String> = (0..row_count)
                    .map(|_| {
                        let pk_ph = self.parameter_placeholder(idx);
                        idx += 1;
                        let val_ph = self.parameter_placeholder(idx);
                        idx += 1;
                        format!("WHEN {} THEN {}", pk_ph, val_ph)
                    })
                    .collect();
                format!(
                    "{} = CASE {} {} END",
                    self.quote_identifier(col),
                    self.quote_identifier(pk_col),
                    whens.join(" ")
                )
            })
            .collect();
        format!(
            "UPDATE {} SET {} WHERE {}",
            self.quote_identifier(table),
            sets.join(", "),
            where_clause
        )
    }
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

    /// Whether `insert_batch` includes a `RETURNING *` clause (PostgreSQL).
    /// When true, `execute_inserts` uses `query()` to read back generated PKs
    /// directly from the INSERT result set.
    fn supports_returning(&self) -> bool {
        false
    }

    /// SQL that retrieves the auto-increment key generated by the most recent
    /// batch INSERT. Returns `None` when the dialect uses `RETURNING` instead.
    /// - SQLite: `SELECT last_insert_rowid()` (returns the LAST rowid)
    /// - MySQL: `SELECT LAST_INSERT_ID()` (returns the FIRST generated ID)
    fn last_insert_id_sql(&self) -> Option<&'static str> {
        None
    }

    /// Whether `last_insert_id_sql()` returns the FIRST (MySQL) or LAST
    /// (SQLite) generated ID in a batch INSERT. The executor uses this to
    /// compute the full key sequence: `first_id..first_id+N` or
    /// `last_id-N+1..last_id`.
    fn last_insert_id_returns_first(&self) -> bool {
        true
    }

    /// Generates a batch UPSERT statement (`row_count` value groups).
    ///
    /// - SQLite/PostgreSQL: `INSERT INTO t (cols) VALUES (...) ON CONFLICT(conflict_cols) DO UPDATE SET ...`
    /// - MySQL: `INSERT INTO t (cols) VALUES (...) ON DUPLICATE KEY UPDATE ...`
    ///
    /// `columns` are the INSERT columns (excluding auto-increment).
    /// `conflict_cols` are the PK (or unique constraint) column names used as
    /// the conflict target. The UPDATE SET clause is generated for all
    /// `columns` that are NOT in `conflict_cols`.
    fn upsert_batch(
        &self,
        table: &str,
        columns: &[&str],
        conflict_cols: &[&str],
        row_count: usize,
    ) -> String {
        let _ = (table, columns, conflict_cols, row_count);
        String::new()
    }
}

/// ANSI SQL transaction isolation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

/// Trait for async database connections.
#[async_trait]
pub trait IAsyncConnection: Send + Sync {
    /// Executes a query with parameters and returns the number of affected rows.
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64>;
    /// Executes a query with parameters and returns rows.
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<DbValue>>>;
    /// Begins a transaction.
    async fn begin_transaction(&mut self) -> EFResult<()>;
    /// Commits the current transaction.
    async fn commit_transaction(&mut self) -> EFResult<()>;
    /// Rolls back the current transaction.
    async fn rollback_transaction(&mut self) -> EFResult<()>;
    /// Creates a savepoint within the current transaction.
    async fn create_savepoint(&mut self, name: &str) -> EFResult<()>;
    /// Releases (commits) a previously created savepoint, discarding its rollback point.
    async fn release_savepoint(&mut self, name: &str) -> EFResult<()>;
    /// Rolls back to the named savepoint, preserving the outer transaction.
    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()>;
    /// Sets the isolation level of the current transaction.
    /// Must be called after `begin_transaction` and before any query.
    async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()>;

    /// Sets the slow query threshold for this connection.
    ///
    /// Only available when the `tracing` feature is enabled on the core
    /// crate. Default implementation is a no-op; provider connections
    /// override to store the threshold for `QueryGuard` comparison.
    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&mut self, _threshold: std::time::Duration) {}
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

    /// Sets the slow query threshold for all connections from this provider.
    ///
    /// Only available when the `tracing` feature is enabled on the core
    /// crate. Default implementation is a no-op; providers override to
    /// store the threshold and pass it to connections on acquisition.
    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&self, _threshold: std::time::Duration) {}
}
