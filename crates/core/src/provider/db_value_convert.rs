//! `TryFrom<DbValue>` impls for primitive and feature-gated types.

use super::db_value::{DbValue, DbValueConvertError};

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
