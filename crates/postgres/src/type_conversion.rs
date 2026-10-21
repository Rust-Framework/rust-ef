use bytes::BytesMut;
use rust_ef::provider::DbValue;
use std::error::Error;
use tokio_postgres::types::{to_sql_checked, IsNull, ToSql, Type};

/// Numeric parameter that adapts to PostgreSQL's inferred parameter type.
///
/// PostgreSQL infers a parameter type from its SQL context (for example,
/// `$1` compared with a `double precision` column is `FLOAT8`). A bare `i32`
/// `ToSql` rejects `FLOAT8`, producing "error serializing parameter 0" for a
/// natural Rust literal such as `value == 3`. This wrapper casts numeric
/// values to the server's expected type.
#[derive(Debug)]
enum PgNumeric {
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl ToSql for PgNumeric {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        match self {
            PgNumeric::I16(v) => match *ty {
                Type::INT4 => (*v as i32).to_sql(ty, out),
                Type::INT8 => (*v as i64).to_sql(ty, out),
                Type::FLOAT4 => (*v as f32).to_sql(ty, out),
                Type::FLOAT8 => (*v as f64).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
            PgNumeric::I32(v) => match *ty {
                Type::INT2 => (*v as i16).to_sql(ty, out),
                Type::INT8 => (*v as i64).to_sql(ty, out),
                Type::FLOAT4 => (*v as f32).to_sql(ty, out),
                Type::FLOAT8 => (*v as f64).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
            PgNumeric::I64(v) => match *ty {
                Type::INT2 => (*v as i16).to_sql(ty, out),
                Type::INT4 => (*v as i32).to_sql(ty, out),
                Type::FLOAT4 => (*v as f32).to_sql(ty, out),
                Type::FLOAT8 => (*v as f64).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
            PgNumeric::F32(v) => match *ty {
                Type::INT2 => (*v as i16).to_sql(ty, out),
                Type::INT4 => (*v as i32).to_sql(ty, out),
                Type::INT8 => (*v as i64).to_sql(ty, out),
                Type::FLOAT8 => (*v as f64).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
            PgNumeric::F64(v) => match *ty {
                Type::INT2 => (*v as i16).to_sql(ty, out),
                Type::INT4 => (*v as i32).to_sql(ty, out),
                Type::INT8 => (*v as i64).to_sql(ty, out),
                Type::FLOAT4 => (*v as f32).to_sql(ty, out),
                _ => v.to_sql(ty, out),
            },
        }
    }

    fn accepts(ty: &Type) -> bool {
        matches!(
            *ty,
            Type::INT2 | Type::INT4 | Type::INT8 | Type::FLOAT4 | Type::FLOAT8
        )
    }

    to_sql_checked!();
}

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
            DbValue::I16(n) => Box::new(PgNumeric::I16(*n)),
            DbValue::I32(n) => Box::new(PgNumeric::I32(*n)),
            DbValue::I64(n) => Box::new(PgNumeric::I64(*n)),
            DbValue::F32(n) => Box::new(PgNumeric::F32(*n)),
            DbValue::F64(n) => Box::new(PgNumeric::F64(*n)),
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
