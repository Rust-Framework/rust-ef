//! Database introspection for MySQL via `information_schema`.

use rust_ef::error::EfResult;
use sqlx::Row;

/// Column information from database introspection.
#[derive(Debug, Clone)]
pub struct DbColumn {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub is_primary_key: bool,
    pub max_length: Option<usize>,
}

/// Table information from database introspection.
#[derive(Debug, Clone)]
pub struct DbTable {
    pub name: String,
    pub columns: Vec<DbColumn>,
}

/// Reads tables and columns from a MySQL database.
pub async fn introspect_mysql(connection_string: &str) -> EfResult<Vec<DbTable>> {
    let pool = sqlx::MySqlPool::connect(connection_string)
        .await
        .map_err(|e| rust_ef::error::EfError::Connection(format!("MySQL connect failed: {e}")))?;

    let table_rows = sqlx::query(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' \
         AND table_name NOT LIKE '__ef_%' \
         ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| rust_ef::error::EfError::Query(format!("Table query error: {e}")))?;

    let mut tables = Vec::new();
    for table_row in table_rows {
        let table_name: String = table_row.try_get(0).map_err(map_row_err)?;

        let col_rows = sqlx::query(
            "SELECT c.column_name, c.data_type, c.is_nullable, c.character_maximum_length, \
             CASE WHEN k.column_name IS NOT NULL THEN 1 ELSE 0 END AS is_pk \
             FROM information_schema.columns c \
             LEFT JOIN information_schema.key_column_usage k \
               ON c.table_schema = k.table_schema \
              AND c.table_name = k.table_name \
              AND c.column_name = k.column_name \
              AND k.constraint_name = 'PRIMARY' \
             WHERE c.table_schema = DATABASE() AND c.table_name = ? \
             ORDER BY c.ordinal_position",
        )
        .bind(&table_name)
        .fetch_all(&pool)
        .await
        .map_err(|e| rust_ef::error::EfError::Query(format!("Column query error: {e}")))?;

        let mut columns = Vec::new();
        for col_row in col_rows {
            let col_name: String = col_row.try_get(0).map_err(map_row_err)?;
            let data_type: String = col_row.try_get(1).map_err(map_row_err)?;
            let is_nullable_str: String = col_row.try_get(2).map_err(map_row_err)?;
            let max_length: Option<i64> = col_row.try_get(3).ok();
            let is_pk: i32 = col_row.try_get(4).map_err(map_row_err)?;

            columns.push(DbColumn {
                name: col_name,
                data_type,
                is_nullable: is_nullable_str == "YES",
                is_primary_key: is_pk != 0,
                max_length: max_length.map(|n| n as usize),
            });
        }

        tables.push(DbTable { name: table_name, columns });
    }

    Ok(tables)
}

fn map_row_err(e: sqlx::Error) -> rust_ef::error::EfError {
    rust_ef::error::EfError::Query(format!("Row read error: {e}"))
}
