use rust_ef::provider::DbValue;
use tokio_postgres::types::Type as PgType;

/// Converts a cell from a `tokio_postgres::Row` into a native `DbValue`.
///
/// PostgreSQL's type checker rejects `try_get` for the wrong Rust type, so a
/// type-tag switch is unnecessary (and brittle when a column OID is reported
/// differently than the value encoding). Trying the supported types in order
/// avoids `Row::get` panics on type mismatches; a SQL NULL matches the first
/// candidate and is returned as `DbValue::Null`.
pub(crate) fn cell_to_db_value(
    row: &tokio_postgres::Row,
    col_idx: usize,
    _pg_type: &PgType,
) -> DbValue {
    macro_rules! try_value {
        ($ty:ty, $variant:path) => {
            if let Ok(value) = row.try_get::<_, Option<$ty>>(col_idx) {
                return value.map($variant).unwrap_or(DbValue::Null);
            }
        };
    }

    try_value!(bool, DbValue::Bool);
    try_value!(i16, DbValue::I16);
    try_value!(i32, DbValue::I32);
    try_value!(i64, DbValue::I64);
    try_value!(f32, DbValue::F32);
    try_value!(f64, DbValue::F64);
    try_value!(chrono::NaiveDateTime, DbValue::NaiveDateTime);
    try_value!(chrono::NaiveDate, DbValue::NaiveDate);
    try_value!(chrono::DateTime<chrono::Utc>, DbValue::DateTime);
    try_value!(uuid::Uuid, DbValue::Uuid);
    try_value!(String, DbValue::String);
    try_value!(Vec<u8>, DbValue::Bytes);

    DbValue::Null
}
