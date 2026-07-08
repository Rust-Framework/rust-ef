use tokio_postgres::types::Type as PgType;

/// Converts a cell from a `tokio_postgres::Row` into a `String` suitable for
/// the `IFromRow::from_row(&[String])` interface.
///
/// This dispatches on the column's PostgreSQL type OID so that native
/// `TIMESTAMPTZ`/`TIMESTAMP`/`DATE`/`UUID` columns are read via their
/// `FromSql` impls (enabled by `with-chrono-0_4`/`with-uuid-1`) and then
/// serialized to a canonical string form. This avoids the v1.0 "silent
/// error swallowing" bug where `try_get::<_, String>` failed on native
/// types and returned `"NULL"`.
pub(crate) fn cell_to_string(
    row: &tokio_postgres::Row,
    col_idx: usize,
    pg_type: &PgType,
) -> String {
    use tokio_postgres::types::FromSql;
    match *pg_type {
        PgType::TIMESTAMPTZ => {
            let opt: Option<chrono::DateTime<chrono::Utc>> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            match opt {
                Some(dt) => dt.to_rfc3339(),
                None => "NULL".to_string(),
            }
        }
        PgType::TIMESTAMP => {
            let opt: Option<chrono::NaiveDateTime> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            match opt {
                Some(ndt) => ndt.to_string(),
                None => "NULL".to_string(),
            }
        }
        PgType::DATE => {
            let opt: Option<chrono::NaiveDate> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            match opt {
                Some(nd) => nd.to_string(),
                None => "NULL".to_string(),
            }
        }
        PgType::UUID => {
            let opt: Option<uuid::Uuid> =
                FromSql::from_sql_nullable(pg_type, row.get(col_idx)).ok();
            match opt {
                Some(u) => u.to_string(),
                None => "NULL".to_string(),
            }
        }
        // For all other types (TEXT, INTEGER, BIGINT, BOOLEAN, NUMERIC, etc.)
        // the `String` `FromSql` impl works correctly via the binary protocol.
        _ => row
            .try_get::<_, Option<String>>(col_idx)
            .ok()
            .flatten()
            .unwrap_or_else(|| "NULL".to_string()),
    }
}
