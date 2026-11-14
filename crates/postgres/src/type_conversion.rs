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

/// A typed-null parameter for PostgreSQL.
///
/// PostgreSQL infers a parameter's type from its SQL context (`$1` assigned to
/// a `BIGINT` column is `INT8`). `None::<String>` earlier described a `TEXT`
/// parameter, so binding it to a nullable `BIGINT`/`NUMERIC`/`TIMESTAMPTZ`/...
/// failed with "error serializing parameter N" before the statement ran
/// (EF-PG-01). A real NULL has no representation bytes at all: reporting
/// `IsNull::Yes` for every accepted type is exactly what the binary protocol
/// expects, and the server uses the inferred type.
#[derive(Debug)]
struct PgNull;

impl ToSql for PgNull {
    fn to_sql(
        &self,
        _ty: &Type,
        _out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn Error + Sync + Send>> {
        Ok(IsNull::Yes)
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

/// Converts `DbValue` parameters into boxed `ToSql` trait objects for
/// `tokio_postgres`.
///
/// Native variants (`DateTime`/`NaiveDateTime`/`NaiveDate`/`Uuid`) are bound
/// directly via `tokio_postgres`'s binary protocol (enabled by the
/// `with-chrono-0_4`/`with-uuid-1` features); `Decimal` uses `rust_decimal`'s
/// `db-tokio-postgres` binary NUMERIC adapter (enabled by this crate's
/// dependency feature).
///
/// `DbValue::Null` uses the private `PgNull` adapter so a `None` value can be written to any
/// nullable column type, not just `TEXT`.
pub fn db_values_to_pg_params(params: &[DbValue]) -> Vec<Box<dyn ToSql + Sync + Send>> {
    params
        .iter()
        .map(|v| match v {
            DbValue::Null => Box::new(PgNull) as Box<dyn ToSql + Sync + Send>,
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
            // `rust_decimal`'s `db-tokio-postgres` feature implements the
            // binary NUMERIC `ToSql`/`FromSql`; binding the decimal directly is
            // required because a `String` parameter is rejected for a NUMERIC
            // slot by the binary protocol (it accepts only TEXT/VARCHAR/...).
            DbValue::Decimal(d) => Box::new(*d),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// EF-PG-01 regression: a NULL parameter must serialize for every type a
    /// nullable column can have, not only `TEXT`.
    #[test]
    fn null_binds_to_every_inferred_parameter_type() {
        let params = db_values_to_pg_params(&[DbValue::Null]);
        let types = [
            Type::INT2,
            Type::INT4,
            Type::INT8,
            Type::NUMERIC,
            Type::BOOL,
            Type::DATE,
            Type::TIMESTAMP,
            Type::TIMESTAMPTZ,
            Type::UUID,
            Type::TEXT,
            Type::BYTEA,
            Type::FLOAT8,
        ];
        for ty in types {
            let mut out = BytesMut::new();
            let result = params[0]
                .to_sql_checked(&ty, &mut out)
                .unwrap_or_else(|error| panic!("NULL failed for {ty:?}: {error}"));
            assert!(
                matches!(result, IsNull::Yes),
                "NULL must be null for {ty:?}"
            );
            assert!(out.is_empty(), "NULL must not write bytes for {ty:?}");
        }
    }

    /// A decimal must bind natively to NUMERIC; a string parameter is rejected
    /// by the binary protocol for a NUMERIC slot.
    #[test]
    fn decimal_binds_natively_to_numeric() {
        let decimal = rust_decimal::Decimal::new(12345, 2); // 123.45
        let params = db_values_to_pg_params(&[DbValue::Decimal(decimal)]);
        let mut out = BytesMut::new();
        let result = params[0]
            .to_sql_checked(&Type::NUMERIC, &mut out)
            .expect("Decimal must serialize for NUMERIC");
        assert!(matches!(result, IsNull::No));
        assert!(!out.is_empty());
    }

    /// The typed wrapper still adapts numeric values to the inferred type.
    #[test]
    fn numeric_values_still_adapt() {
        let params = db_values_to_pg_params(&[DbValue::I64(7)]);
        let mut out = BytesMut::new();
        let result = params[0]
            .to_sql_checked(&Type::FLOAT8, &mut out)
            .expect("i64 -> FLOAT8");
        assert!(matches!(result, IsNull::No));
        assert!(!out.is_empty());
    }
}
