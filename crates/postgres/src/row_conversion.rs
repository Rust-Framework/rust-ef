use rust_ef::provider::DbValue;
use tokio_postgres::types::Type as PgType;

/// Converts a cell from a `tokio_postgres::Row` into a native `DbValue`.
///
/// Dispatches on the column's PostgreSQL type OID so that native
/// `TIMESTAMPTZ`/`TIMESTAMP`/`DATE`/`UUID`/`BOOL`/`INT2`/`INT4`/`INT8`/
/// `FLOAT4`/`FLOAT8`/`BYTEA` columns are read via their `FromSql` impls
/// (enabled by `with-chrono-0_4`/`with-uuid-1`) and returned as the
/// corresponding `DbValue` variant directly — no String round-trip.
///
/// NUMERIC / TEXT / VARCHAR / CHAR / JSON / etc. fall back to the `String`
/// `FromSql` impl (works correctly via the binary protocol).
pub(crate) fn cell_to_db_value(
    row: &tokio_postgres::Row,
    col_idx: usize,
    pg_type: &PgType,
) -> DbValue {
    use tokio_postgres::types::FromSql;
    match *pg_type {
        PgType::BOOL => {
            let opt: Option<bool> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::Bool).unwrap_or(DbValue::Null)
        }
        PgType::INT2 => {
            let opt: Option<i16> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::I16).unwrap_or(DbValue::Null)
        }
        PgType::INT4 => {
            let opt: Option<i32> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::I32).unwrap_or(DbValue::Null)
        }
        PgType::INT8 => {
            let opt: Option<i64> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::I64).unwrap_or(DbValue::Null)
        }
        PgType::FLOAT4 => {
            let opt: Option<f32> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::F32).unwrap_or(DbValue::Null)
        }
        PgType::FLOAT8 => {
            let opt: Option<f64> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::F64).unwrap_or(DbValue::Null)
        }
        PgType::BYTEA => {
            let opt: Option<Vec<u8>> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::Bytes).unwrap_or(DbValue::Null)
        }
        PgType::TIMESTAMPTZ => {
            let opt: Option<chrono::DateTime<chrono::Utc>> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::DateTime).unwrap_or(DbValue::Null)
        }
        PgType::TIMESTAMP => {
            let opt: Option<chrono::NaiveDateTime> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::NaiveDateTime).unwrap_or(DbValue::Null)
        }
        PgType::DATE => {
            let opt: Option<chrono::NaiveDate> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::NaiveDate).unwrap_or(DbValue::Null)
        }
        PgType::UUID => {
            let opt: Option<uuid::Uuid> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::Uuid).unwrap_or(DbValue::Null)
        }
        // NUMERIC / TEXT / VARCHAR / CHAR / JSON / etc. — String FromSql works
        // correctly via the binary protocol. NUMERIC returns a string form
        // that round-trips losslessly through PG's NUMERIC type.
        _ => {
            let opt: Option<String> = FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            opt.map(DbValue::String).unwrap_or(DbValue::Null)
        }
    }
}
