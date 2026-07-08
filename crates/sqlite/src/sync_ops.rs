use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::DbValue;

/// Synchronous execute logic shared by both connection modes (pooled / shared).
pub(crate) fn execute_sync(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[DbValue],
) -> EFResult<u64> {
    let rp = crate::type_conversion::to_rusqlite_params(params);
    let refs: Vec<&dyn rusqlite::types::ToSql> = rp
        .iter()
        .map(|v| v as &dyn rusqlite::types::ToSql)
        .collect();
    conn.execute(sql, refs.as_slice())
        .map(|c| c as u64)
        .map_err(|e| EFError::query(format!("Execution error: {}", e)))
}

/// Synchronous query logic shared by both connection modes (pooled / shared).
pub(crate) fn query_sync(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[DbValue],
) -> EFResult<Vec<Vec<DbValue>>> {
    let rp = crate::type_conversion::to_rusqlite_params(params);
    let refs: Vec<&dyn rusqlite::types::ToSql> = rp
        .iter()
        .map(|v| v as &dyn rusqlite::types::ToSql)
        .collect();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| EFError::query(format!("Prepare error: {}", e)))?;
    let cc = stmt.column_count();
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            let mut vals = Vec::with_capacity(cc);
            for i in 0..cc {
                let val = match row.get_ref(i) {
                    Ok(rusqlite::types::ValueRef::Null) => DbValue::Null,
                    Ok(rusqlite::types::ValueRef::Integer(n)) => DbValue::I64(n),
                    Ok(rusqlite::types::ValueRef::Real(x)) => DbValue::F64(x),
                    Ok(rusqlite::types::ValueRef::Text(bytes)) => {
                        DbValue::String(String::from_utf8_lossy(bytes).into_owned())
                    }
                    Ok(rusqlite::types::ValueRef::Blob(bytes)) => DbValue::Bytes(bytes.to_vec()),
                    Err(_) => DbValue::Null,
                };
                vals.push(val);
            }
            Ok(vals)
        })
        .map_err(|e| EFError::query(format!("Query error: {}", e)))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| EFError::query(format!("Row read error: {}", e)))?);
    }
    Ok(result)
}
