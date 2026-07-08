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
        .map_err(|e| EFError::Query(format!("Execution error: {}", e)))
}

/// Synchronous query logic shared by both connection modes (pooled / shared).
pub(crate) fn query_sync(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[DbValue],
) -> EFResult<Vec<Vec<String>>> {
    let rp = crate::type_conversion::to_rusqlite_params(params);
    let refs: Vec<&dyn rusqlite::types::ToSql> = rp
        .iter()
        .map(|v| v as &dyn rusqlite::types::ToSql)
        .collect();
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| EFError::Query(format!("Prepare error: {}", e)))?;
    let cc = stmt.column_count();
    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            let mut vals = Vec::with_capacity(cc);
            for i in 0..cc {
                vals.push(
                    row.get::<_, String>(i)
                        .or_else(|_| row.get::<_, i64>(i).map(|n| n.to_string()))
                        .or_else(|_| row.get::<_, f64>(i).map(|n| n.to_string()))
                        .unwrap_or_else(|_| "NULL".to_string()),
                );
            }
            Ok(vals)
        })
        .map_err(|e| EFError::Query(format!("Query error: {}", e)))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| EFError::Query(format!("Row read error: {}", e)))?);
    }
    Ok(result)
}
