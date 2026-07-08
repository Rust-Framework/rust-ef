use rust_ef::provider::DbValue;
use sqlx::Row;

/// Converts a cell from a `sqlx::mysql::MySqlRow` into a native `DbValue`.
///
/// Dispatches by attempting `try_get` on common Rust types in a specific
/// order (bool → i16 → i32 → i64 → f32 → f64 → NaiveDateTime → NaiveDate →
/// Uuid → String → Vec<u8>). This mirrors the PostgreSQL provider's
/// `cell_to_db_value` approach and returns native `DbValue` variants directly
/// (no String round-trip), eliminating the v1.5 silent-data-loss path where
/// non-String columns were stringified and re-parsed.
///
/// `Ok(None)` from `try_get::<Option<T>>` indicates a SQL NULL — returned as
/// `DbValue::Null`. `Err` indicates the column's native type is not
/// representable as `T`, so we fall through to the next candidate type.
pub(crate) fn cell_to_db_value(row: &sqlx::mysql::MySqlRow, col_idx: usize) -> DbValue {
    // bool (TINYINT(1)) — must precede integer types, since MySQL booleans
    // are stored as 0/1 and would otherwise be matched by i16/i32/i64.
    if let Ok(v) = row.try_get::<Option<bool>, _>(col_idx) {
        return v.map(DbValue::Bool).unwrap_or(DbValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<i16>, _>(col_idx) {
        return v.map(DbValue::I16).unwrap_or(DbValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(col_idx) {
        return v.map(DbValue::I32).unwrap_or(DbValue::Null);
    }
    // i64 covers BIGINT (signed). UNSIGNED BIGINT (u64) may overflow i64 but
    // sqlx returns it as i64 when possible; values > i64::MAX are rare in
    // practice and will be caught by the String fallback below.
    if let Ok(v) = row.try_get::<Option<i64>, _>(col_idx) {
        return v.map(DbValue::I64).unwrap_or(DbValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<f32>, _>(col_idx) {
        return v.map(DbValue::F32).unwrap_or(DbValue::Null);
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(col_idx) {
        return v.map(DbValue::F64).unwrap_or(DbValue::Null);
    }
    // NaiveDateTime (DATETIME, TIMESTAMP)
    #[cfg(feature = "chrono")]
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(col_idx) {
        return v.map(DbValue::NaiveDateTime).unwrap_or(DbValue::Null);
    }
    // NaiveDate (DATE)
    #[cfg(feature = "chrono")]
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(col_idx) {
        return v.map(DbValue::NaiveDate).unwrap_or(DbValue::Null);
    }
    // Uuid (CHAR(36))
    #[cfg(feature = "uuid")]
    if let Ok(v) = row.try_get::<Option<uuid::Uuid>, _>(col_idx) {
        return v.map(DbValue::Uuid).unwrap_or(DbValue::Null);
    }
    // String (VARCHAR, TEXT, CHAR) — also catches DECIMAL since MySQL
    // returns DECIMAL as string to preserve precision.
    if let Ok(v) = row.try_get::<Option<String>, _>(col_idx) {
        return v.map(DbValue::String).unwrap_or(DbValue::Null);
    }
    // bytes (BLOB, BINARY)
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(col_idx) {
        return v.map(DbValue::Bytes).unwrap_or(DbValue::Null);
    }
    // Unknown / unsupported type — last resort.
    // M3.1 will add a tracing::warn! here (tracing feature gated).
    DbValue::Null
}
