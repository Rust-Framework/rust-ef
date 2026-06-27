use async_trait::async_trait;
use rust_ef::error::{EFError, EFResult};
use rust_ef::provider::{DbValue, IAsyncConnection};
use tokio_postgres::types::ToSql;
use tokio_postgres::types::Type as PgType;

pub struct PostgresConnection {
    pub(crate) client: deadpool_postgres::Client,
}

/// Converts a cell from a `tokio_postgres::Row` into a `String` suitable for
/// the `IFromRow::from_row(&[String])` interface.
///
/// This dispatches on the column's PostgreSQL type OID so that native
/// `TIMESTAMPTZ`/`TIMESTAMP`/`DATE`/`UUID` columns are read via their
/// `FromSql` impls (enabled by `with-chrono-0_4`/`with-uuid-1`) and then
/// serialized to a canonical string form. This avoids the v1.0 "silent
/// error swallowing" bug where `try_get::<_, String>` failed on native
/// types and returned `"NULL"`.
fn cell_to_string(row: &tokio_postgres::Row, col_idx: usize, pg_type: &PgType) -> String {
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

#[async_trait]
impl IAsyncConnection for PostgresConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        let pgp = crate::type_conversion::db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pgp
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        self.client
            .execute(sql, &refs)
            .await
            .map_err(|e| EFError::Query(format!("Execution error: {}", e)))
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>> {
        let pgp = crate::type_conversion::db_values_to_pg_params(params);
        let refs: Vec<&(dyn ToSql + Sync)> = pgp
            .iter()
            .map(|p| p.as_ref() as &(dyn ToSql + Sync))
            .collect();
        let rows = self
            .client
            .query(sql, &refs)
            .await
            .map_err(|e| EFError::Query(format!("Query error: {}", e)))?;
        let columns: Vec<&tokio_postgres::Column> = if !rows.is_empty() {
            rows[0].columns().iter().collect()
        } else {
            Vec::new()
        };
        let result = rows
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, col)| cell_to_string(row, i, col.type_()))
                    .collect()
            })
            .collect();
        Ok(result)
    }

    async fn begin_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("BEGIN")
            .await
            .map_err(|e| EFError::Transaction(format!("BEGIN failed: {}", e)))?;
        Ok(())
    }

    async fn commit_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("COMMIT")
            .await
            .map_err(|e| EFError::Transaction(format!("COMMIT failed: {}", e)))?;
        Ok(())
    }

    async fn rollback_transaction(&mut self) -> EFResult<()> {
        self.client
            .simple_query("ROLLBACK")
            .await
            .map_err(|e| EFError::Transaction(format!("ROLLBACK failed: {}", e)))?;
        Ok(())
    }
}
