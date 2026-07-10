//! `DbValueKey` — hashable, `Eq`-satisfying key form of `DbValue`.
//!
//! `DbValue` cannot derive `Eq`/`Hash` because `f32`/`f64` are not `Eq` (NaN
//! != NaN). `DbValueKey` normalizes every variant into a hashable shape:
//!   - Integers (I16/I32/I64) collapse to `Int(i64)` — zero allocation.
//!   - Floats (F32/F64) collapse to `FloatBits(u64)` via `to_bits()` — zero
//!     allocation and correct hash semantics (NaN hashes consistently).
//!   - String/Bytes keep their owned form (one clone, but no `format!`
//!     machinery overhead).
//!
//! Used as the HashMap key in navigation loading (`group_rows`, `index_rows`,
//! `group_join_rows`) to eliminate per-key `format!("{}", v)` allocation.

use super::db_value::DbValue;

#[cfg(feature = "chrono")]
use chrono::Datelike;

/// Hashable key derived from a `&DbValue`.
///
/// See the module docs for the normalization rationale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DbValueKey {
    Null,
    Bool(bool),
    /// I16/I32/I64 unified — the most common PK/FK type, zero allocation.
    Int(i64),
    /// F32/F64 unified via `to_bits()` — correct hash semantics for NaN.
    FloatBits(u64),
    Str(String),
    Bytes(Vec<u8>),
    /// UTC timestamp millis — zero allocation.
    #[cfg(feature = "chrono")]
    DateTime(i64),
    /// Naive timestamp millis — zero allocation.
    #[cfg(feature = "chrono")]
    NaiveDateTime(i64),
    /// Days from Unix epoch — zero allocation.
    #[cfg(feature = "chrono")]
    NaiveDate(i32),
    /// 128-bit UUID — zero allocation.
    #[cfg(feature = "uuid")]
    Uuid(u128),
    /// Decimal serialized to string (rust_decimal lacks Hash).
    #[cfg(feature = "decimal")]
    Decimal(String),
}

impl From<&DbValue> for DbValueKey {
    fn from(v: &DbValue) -> Self {
        match v {
            DbValue::Null => DbValueKey::Null,
            DbValue::Bool(b) => DbValueKey::Bool(*b),
            DbValue::I16(n) => DbValueKey::Int(*n as i64),
            DbValue::I32(n) => DbValueKey::Int(*n as i64),
            DbValue::I64(n) => DbValueKey::Int(*n),
            DbValue::F32(f) => DbValueKey::FloatBits(f.to_bits() as u64),
            DbValue::F64(f) => DbValueKey::FloatBits(f.to_bits()),
            DbValue::String(s) => DbValueKey::Str(s.clone()),
            DbValue::Bytes(b) => DbValueKey::Bytes(b.clone()),
            #[cfg(feature = "chrono")]
            DbValue::DateTime(dt) => DbValueKey::DateTime(dt.timestamp_millis()),
            #[cfg(feature = "chrono")]
            DbValue::NaiveDateTime(ndt) => {
                DbValueKey::NaiveDateTime(ndt.and_utc().timestamp_millis())
            }
            #[cfg(feature = "chrono")]
            DbValue::NaiveDate(nd) => DbValueKey::NaiveDate(nd.num_days_from_ce()),
            #[cfg(feature = "uuid")]
            DbValue::Uuid(u) => DbValueKey::Uuid(u.as_u128()),
            #[cfg(feature = "decimal")]
            DbValue::Decimal(d) => DbValueKey::Decimal(d.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_variants_collapse_to_int() {
        assert_eq!(DbValueKey::from(&DbValue::I16(7)), DbValueKey::Int(7));
        assert_eq!(DbValueKey::from(&DbValue::I32(7)), DbValueKey::Int(7));
        assert_eq!(DbValueKey::from(&DbValue::I64(7)), DbValueKey::Int(7));
    }

    #[test]
    fn float_variants_use_bits() {
        let key = DbValueKey::from(&DbValue::F64(2.5));
        assert_eq!(key, DbValueKey::FloatBits(2.5f64.to_bits()));
        let key32 = DbValueKey::from(&DbValue::F32(1.5));
        assert_eq!(key32, DbValueKey::FloatBits(1.5f32.to_bits() as u64));
    }

    #[test]
    fn nan_hashes_consistently() {
        let nan = DbValue::F64(f64::NAN);
        let k1 = DbValueKey::from(&nan);
        let k2 = DbValueKey::from(&nan);
        assert_eq!(k1, k2, "NaN must produce equal keys for HashMap lookup");
    }

    #[test]
    fn string_and_bytes_clone() {
        let s = DbValueKey::from(&DbValue::String("hello".into()));
        assert_eq!(s, DbValueKey::Str("hello".into()));
        let b = DbValueKey::from(&DbValue::Bytes(vec![1, 2, 3]));
        assert_eq!(b, DbValueKey::Bytes(vec![1, 2, 3]));
    }

    #[test]
    fn null_key() {
        assert_eq!(DbValueKey::from(&DbValue::Null), DbValueKey::Null);
    }

    #[test]
    fn hashmap_lookup_with_integer_key() {
        let mut map = std::collections::HashMap::new();
        map.insert(DbValueKey::from(&DbValue::I32(42)), "answer");
        assert_eq!(
            map.get(&DbValueKey::from(&DbValue::I64(42))),
            Some(&"answer"),
            "I32 and I64 with same value must collide in the map"
        );
    }

    #[cfg(feature = "chrono")]
    #[test]
    fn datetime_key_is_stable() {
        let dt = chrono::Utc::now();
        let k1 = DbValueKey::from(&DbValue::DateTime(dt));
        let k2 = DbValueKey::from(&DbValue::DateTime(dt));
        assert_eq!(k1, k2);
    }

    #[cfg(feature = "uuid")]
    #[test]
    fn uuid_key_roundtrips() {
        let u = uuid::Uuid::new_v4();
        let key = DbValueKey::from(&DbValue::Uuid(u));
        assert_eq!(key, DbValueKey::Uuid(u.as_u128()));
    }
}
