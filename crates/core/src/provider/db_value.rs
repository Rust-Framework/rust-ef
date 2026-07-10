//! `DbValue` — typed database parameter value and conversion error type.

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
