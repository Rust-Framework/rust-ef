//! `TryFrom<DbValue>` impls for primitive and feature-gated types.

use super::db_value::{DbValue, DbValueConvertError};

// --- Decimal (NUMERIC) -> primitive conversions -----------------------------
//
// PostgreSQL returns `NUMERIC` for `SUM(bigint)` / `AVG(int)`, and the
// providers decode `NUMERIC` as `DbValue::Decimal`. The integer conversions
// therefore accept a `Decimal` only when it is a whole number (a fractional
// `SUM`/`AVG` decoded into `i64` is a caller error, reported as a typed
// conversion failure), while the float conversions accept any `Decimal`.
#[cfg(feature = "decimal")]
mod decimal_conv {
    use rust_decimal::prelude::ToPrimitive;
    use rust_decimal::Decimal;

    pub(super) fn to_i64(value: Decimal) -> Option<i64> {
        let whole = value.to_i64()?;
        (Decimal::from(whole) == value).then_some(whole)
    }

    pub(super) fn to_i32(value: Decimal) -> Option<i32> {
        let whole = value.to_i32()?;
        (Decimal::from(whole) == value).then_some(whole)
    }

    pub(super) fn to_i16(value: Decimal) -> Option<i16> {
        let whole = value.to_i16()?;
        (Decimal::from(whole) == value).then_some(whole)
    }

    pub(super) fn to_i8(value: Decimal) -> Option<i8> {
        let whole = value.to_i8()?;
        (Decimal::from(whole) == value).then_some(whole)
    }

    pub(super) fn to_u32(value: Decimal) -> Option<u32> {
        let whole = value.to_u32()?;
        (Decimal::from(whole) == value).then_some(whole)
    }

    pub(super) fn to_u64(value: Decimal) -> Option<u64> {
        let whole = value.to_u64()?;
        (Decimal::from(whole) == value).then_some(whole)
    }

    pub(super) fn to_f32(value: Decimal) -> Option<f32> {
        value.to_f32()
    }

    pub(super) fn to_f64(value: Decimal) -> Option<f64> {
        value.to_f64()
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_i32(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_i64(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_f64(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_f32(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_i16(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_i8(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_u32(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => decimal_conv::to_u64(d).ok_or(DbValueConvertError {
                source: DbValue::Decimal(d),
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

// ---------------------------------------------------------------------------
// Nullable cells (`Option<T>`)
// ---------------------------------------------------------------------------
//
// A projection row is positional and every cell may be SQL `NULL` (LEFT JOIN
// misses, nullable report columns). `Option<T>` is how a `#[derive(FromRow)]`
// struct or a tuple row expresses that: `NULL` decodes to `None`, any other
// cell decodes through the inner type's `TryFrom<DbValue>`.
macro_rules! impl_optional_cell {
    ($($t:ty),+ $(,)?) => {
        $(
            impl TryFrom<DbValue> for Option<$t> {
                type Error = DbValueConvertError;

                fn try_from(value: DbValue) -> Result<Self, Self::Error> {
                    match value {
                        DbValue::Null => Ok(None),
                        other => <$t>::try_from(other).map(Some),
                    }
                }
            }
        )+
    };
}

impl_optional_cell!(i8, i16, i32, i64, u32, u64, f32, f64, bool, String, Vec<u8>);

#[cfg(feature = "chrono")]
impl_optional_cell!(
    chrono::DateTime<chrono::Utc>,
    chrono::NaiveDateTime,
    chrono::NaiveDate
);

#[cfg(feature = "uuid")]
impl_optional_cell!(uuid::Uuid);

#[cfg(feature = "decimal")]
impl_optional_cell!(rust_decimal::Decimal);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_decodes_to_none_for_every_optional_cell() {
        assert_eq!(Option::<i64>::try_from(DbValue::Null).unwrap(), None);
        assert_eq!(Option::<f64>::try_from(DbValue::Null).unwrap(), None);
        assert_eq!(Option::<bool>::try_from(DbValue::Null).unwrap(), None);
        assert_eq!(Option::<String>::try_from(DbValue::Null).unwrap(), None);
        assert_eq!(Option::<Vec<u8>>::try_from(DbValue::Null).unwrap(), None);
    }

    #[test]
    fn present_cells_decode_through_the_inner_type() {
        assert_eq!(Option::<i64>::try_from(DbValue::I64(7)).unwrap(), Some(7));
        assert_eq!(
            Option::<String>::try_from(DbValue::I32(3)).unwrap(),
            Some("3".to_string())
        );
        assert_eq!(
            Option::<Vec<u8>>::try_from(DbValue::Bytes(vec![1, 2, 3])).unwrap(),
            Some(vec![1, 2, 3])
        );
    }

    #[test]
    fn the_inner_conversion_error_still_surfaces() {
        let error = Option::<i64>::try_from(DbValue::Bool(true)).unwrap_err();
        assert_eq!(error.target_type, "i64");
    }

    #[cfg(feature = "decimal")]
    mod decimal {
        use super::*;
        use rust_decimal::Decimal;

        #[test]
        fn whole_numeric_decodes_into_integers() {
            // PostgreSQL returns NUMERIC for SUM(bigint)/AVG(int).
            assert_eq!(
                i64::try_from(DbValue::Decimal(Decimal::from(42i64))).unwrap(),
                42
            );
            assert_eq!(
                i32::try_from(DbValue::Decimal(Decimal::from(7i32))).unwrap(),
                7
            );
            assert_eq!(
                Option::<i64>::try_from(DbValue::Decimal(Decimal::from(9i64))).unwrap(),
                Some(9)
            );
        }

        #[test]
        fn fractional_numeric_does_not_silently_truncate_into_integers() {
            let fractional = DbValue::Decimal(Decimal::new(1234, 2)); // 12.34
            assert!(i64::try_from(fractional.clone()).is_err());
            assert!(i32::try_from(fractional.clone()).is_err());
            // Floats accept the value (documented lossy path).
            let float = f64::try_from(fractional).unwrap();
            assert!((float - 12.34).abs() < 1e-9);
        }

        #[test]
        fn null_decimal_cell_decodes_to_none() {
            assert_eq!(Option::<Decimal>::try_from(DbValue::Null).unwrap(), None);
        }
    }
}
