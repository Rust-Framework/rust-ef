use rust_ef::provider::DbValue;

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
        };
    }
    query
}
