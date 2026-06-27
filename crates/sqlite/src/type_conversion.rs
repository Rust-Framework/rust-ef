use rust_ef::provider::DbValue;

/// Converts `DbValue` parameters into boxed `ToSql` trait objects for
/// `rusqlite`.
///
/// Native chrono/uuid/decimal variants are collapsed to their canonical
/// string form because SQLite has no native TIMESTAMPTZ/UUID type — TEXT is
/// the correct storage (matching v1.0 behavior). This also keeps the
/// round-trip path symmetric with the read-back path in `connection.rs`,
/// which decodes everything to `String` via `IFromRow::from_row`.
pub fn to_rusqlite_params(params: &[DbValue]) -> Vec<Box<dyn rusqlite::types::ToSql>> {
    params
        .iter()
        .map(|v| match v {
            DbValue::Null => Box::new(None::<String>) as Box<dyn rusqlite::types::ToSql>,
            DbValue::Bool(b) => Box::new(*b),
            DbValue::I16(n) => Box::new(*n),
            DbValue::I32(n) => Box::new(*n),
            DbValue::I64(n) => Box::new(*n),
            DbValue::F32(n) => Box::new(*n as f64),
            DbValue::F64(n) => Box::new(*n),
            DbValue::String(s) => Box::new(s.clone()),
            DbValue::Bytes(b) => Box::new(b.clone()),
            // Collapse native chrono variants to canonical string form.
            // RFC 3339 for DateTime<Utc>, "YYYY-MM-DD HH:MM:SS" for
            // NaiveDateTime, "YYYY-MM-DD" for NaiveDate.
            DbValue::DateTime(dt) => Box::new(dt.to_rfc3339()),
            DbValue::NaiveDateTime(ndt) => Box::new(ndt.to_string()),
            DbValue::NaiveDate(nd) => Box::new(nd.to_string()),
            // UUID hyphenated lowercase form.
            DbValue::Uuid(u) => Box::new(u.to_string()),
            // Decimal canonical string form (preserves full precision).
            DbValue::Decimal(d) => Box::new(d.to_string()),
        })
        .collect()
}
