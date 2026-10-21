use rust_ef::provider::DbValue;
use sqlx::{Column, Row, TypeInfo};

/// Converts a cell from a `sqlx::mysql::MySqlRow` into a native `DbValue`.
///
/// Uses the column type for the ambiguous cases (`BIGINT`, `TINYINT`), then
/// attempts `try_get` on common Rust types. This mirrors the PostgreSQL
/// provider's `cell_to_db_value` approach and returns native `DbValue`
/// variants directly (no String round-trip), eliminating the v1.5
/// silent-data-loss path where non-String columns were stringified and
/// re-parsed.
///
/// `Ok(None)` from `try_get::<Option<T>>` indicates a SQL NULL — returned as
/// `DbValue::Null`. `Err` indicates the column's native type is not
/// representable as `T`, so we fall through to the next candidate type.
pub(crate) fn cell_to_db_value(row: &sqlx::mysql::MySqlRow, col_idx: usize) -> DbValue {
    // BIGINT must be checked before bool: MySQL's bool decoder accepts any
    // integer, so COUNT(*) (BIGINT) would otherwise become Bool(true/false).
    let column_type = row
        .columns()
        .get(col_idx)
        .map(|column| column.type_info().name())
        .unwrap_or("");
    if column_type.starts_with("BIGINT") {
        if let Ok(v) = row.try_get::<Option<i64>, _>(col_idx) {
            return v.map(DbValue::I64).unwrap_or(DbValue::Null);
        }
    }

    // bool (TINYINT(1)) — only for TINYINT. The MySQL bool decoder accepts
    // any integer, so decoding INT/BIGINT as bool corrupts IDs and counts.
    if column_type == "TINYINT" {
        if let Ok(v) = row.try_get::<Option<bool>, _>(col_idx) {
            return v.map(DbValue::Bool).unwrap_or(DbValue::Null);
        }
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
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDateTime>, _>(col_idx) {
        return v.map(DbValue::NaiveDateTime).unwrap_or(DbValue::Null);
    }
    // NaiveDate (DATE)
    if let Ok(v) = row.try_get::<Option<chrono::NaiveDate>, _>(col_idx) {
        return v.map(DbValue::NaiveDate).unwrap_or(DbValue::Null);
    }
    // Uuid (CHAR(36))
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
    // Unknown / unsupported type — last resort. Emit a tracing warning so
    // production users can diagnose silent NULLs from unrecognized column
    // types (e.g. GEOMETRY, JSON when not auto-detected as String).
    #[cfg(feature = "tracing")]
    {
        let col_name = row
            .columns()
            .get(col_idx)
            .map(|c| c.name())
            .unwrap_or("unknown");
        tracing::warn!(
            column = col_name,
            index = col_idx,
            "MySQL column type unrecognized, returning NULL"
        );
    }
    DbValue::Null
}
