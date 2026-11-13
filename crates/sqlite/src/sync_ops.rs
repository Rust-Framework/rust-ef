use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::DbValue;

/// Maps a `rusqlite` error to a typed [`EFError`].
///
/// Unique/primary-key violations are promoted to
/// [`EFError::UniqueViolation`] so callers no longer need to sniff the driver
/// message. SQLite reports the violated columns instead of a constraint name
/// (`UNIQUE constraint failed: order_record.tenant_id, …`), so `constraint` is
/// always `None`; `table` is parsed from the message when present.
fn map_sqlite_error(e: rusqlite::Error) -> EFError {
    if let rusqlite::Error::SqliteFailure(err, message) = &e {
        let unique = err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            || err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY;
        if unique {
            return EFError::UniqueViolation {
                constraint: None,
                table: message.as_deref().and_then(unique_violation_table),
            };
        }
    }
    EFError::query(format!("Execution error: {}", e))
}

/// Extracts the table name from a SQLite unique-constraint message.
///
/// `"UNIQUE constraint failed: order_record.tenant_id, order_record.request_key"`
/// yields `Some("order_record")`.
fn unique_violation_table(message: &str) -> Option<String> {
    let after_colon = message.split_once(':').map_or(message, |(_, rest)| rest);
    let first_column = after_colon.split(',').next()?.trim();
    let (table, _) = first_column.split_once('.')?;
    let table = table.trim();
    if table.is_empty() {
        None
    } else {
        Some(table.to_string())
    }
}

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
        .map_err(map_sqlite_error)
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
        .prepare_cached(sql)
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

#[cfg(test)]
mod tests {
    use super::unique_violation_table;

    #[test]
    fn parses_table_from_composite_unique_message() {
        assert_eq!(
            unique_violation_table(
                "UNIQUE constraint failed: order_record.tenant_id, order_record.request_key"
            )
            .as_deref(),
            Some("order_record")
        );
    }

    #[test]
    fn parses_table_from_primary_key_message() {
        assert_eq!(
            unique_violation_table("UNIQUE constraint failed: product.sku").as_deref(),
            Some("product")
        );
    }

    #[test]
    fn unknown_message_shape_yields_none() {
        assert_eq!(unique_violation_table("UNIQUE constraint failed"), None);
        assert_eq!(unique_violation_table(""), None);
    }
}
