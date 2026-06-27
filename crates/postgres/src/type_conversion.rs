use rust_ef::provider::DbValue;
use tokio_postgres::types::ToSql;

/// Converts `DbValue` parameters into boxed `ToSql` trait objects for
/// `tokio_postgres`.
///
/// Native variants (`DateTime`/`NaiveDateTime`/`NaiveDate`/`Uuid`) are bound
/// directly via `tokio_postgres`'s binary protocol (enabled by the
/// `with-chrono-0_4`/`with-uuid-1` features). `Decimal` is bound as a string
/// since `tokio_postgres` lacks a native `rust_decimal` adapter; PG's `NUMERIC`
/// type accepts string input and round-trips losslessly.
pub fn db_values_to_pg_params(params: &[DbValue]) -> Vec<Box<dyn ToSql + Sync + Send>> {
    params
        .iter()
        .map(|v| match v {
            DbValue::Null => Box::new(None::<String>) as Box<dyn ToSql + Sync + Send>,
            DbValue::Bool(b) => Box::new(*b),
            DbValue::I16(n) => Box::new(*n),
            DbValue::I32(n) => Box::new(*n),
            DbValue::I64(n) => Box::new(*n),
            DbValue::F32(n) => Box::new(*n),
            DbValue::F64(n) => Box::new(*n),
            DbValue::String(s) => Box::new(s.clone()),
            DbValue::Bytes(b) => Box::new(b.clone()),
            DbValue::DateTime(dt) => Box::new(*dt),
            DbValue::NaiveDateTime(ndt) => Box::new(*ndt),
            DbValue::NaiveDate(nd) => Box::new(*nd),
            DbValue::Uuid(u) => Box::new(*u),
            // tokio_postgres has no native rust_decimal ToSql impl; bind as
            // string. PG's NUMERIC column accepts string input and preserves
            // full precision.
            DbValue::Decimal(d) => Box::new(d.to_string()),
        })
        .collect()
}
