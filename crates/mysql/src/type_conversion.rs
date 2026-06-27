use rust_ef::provider::DbValue;

/// Converts `DbValue` parameters into `sqlx` bindings for MySQL.
///
/// Native chrono/uuid/decimal variants are collapsed to their canonical
/// string form. MySQL's DATETIME/CHAR(36)/DECIMAL columns accept string
/// input and preserve precision (matching v1.0 behavior). This keeps the
/// write path symmetric with the read-back path in `connection.rs`, which
/// decodes everything to `String` via `IFromRow::from_row`.
pub fn build_mysql_query<'q>(
    sql: &'q str,
    params: &'q [DbValue],
) -> sqlx::query::Query<'q, sqlx::MySql, sqlx::mysql::MySqlArguments> {
    let mut query = sqlx::query::<sqlx::MySql>(sql);
    for param in params {
        query = match param {
            DbValue::Null => query.bind(None::<String>),
            DbValue::Bool(v) => query.bind(*v),
            DbValue::I16(v) => query.bind(*v),
            DbValue::I32(v) => query.bind(*v),
            DbValue::I64(v) => query.bind(*v),
            DbValue::F32(v) => query.bind(*v),
            DbValue::F64(v) => query.bind(*v),
            DbValue::String(v) => query.bind(v.as_str()),
            DbValue::Bytes(v) => query.bind(v.as_slice()),
            // Collapse native chrono variants to canonical string form.
            DbValue::DateTime(dt) => query.bind(dt.to_rfc3339()),
            DbValue::NaiveDateTime(ndt) => query.bind(ndt.to_string()),
            DbValue::NaiveDate(nd) => query.bind(nd.to_string()),
            // UUID hyphenated lowercase form — fits CHAR(36).
            DbValue::Uuid(u) => query.bind(u.to_string()),
            // Decimal canonical string form — fits DECIMAL(38,18).
            DbValue::Decimal(d) => query.bind(d.to_string()),
        };
    }
    query
}
