//! Database introspection  - ?reads schema from PostgreSQL information_schema.

use rust_ef::error::EFResult;
use tokio_postgres::NoTls;

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

/// Reads all tables and their columns from a PostgreSQL database.
pub async fn introspect_postgres(connection_string: &str) -> EFResult<Vec<DbTable>> {
    let config: tokio_postgres::Config = connection_string
        .parse()
        .map_err(|e| rust_ef::error::EFError::Connection(format!("Invalid connection string: {}", e)))?;

    let (client, connection) = config
        .connect(NoTls)
        .await
        .map_err(|e| rust_ef::error::EFError::Connection(format!("Connection failed: {}", e)))?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("PostgreSQL connection error: {}", e);
        }
    });

    // Query for tables
    let table_rows = client
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
             AND table_name NOT LIKE '__ef_%' \
             ORDER BY table_name",
            &[],
        )
        .await
        .map_err(|e| rust_ef::error::EFError::Query(format!("Table query error: {}", e)))?;

    let mut tables = Vec::new();
    for table_row in &table_rows {
        let table_name: String = table_row.get(0);

        // Query for columns
        let col_rows = client
            .query(
                "SELECT c.column_name, c.data_type, c.is_nullable, c.character_maximum_length, \
                 CASE WHEN pk.column_name IS NOT NULL THEN true ELSE false END AS is_pk \
                 FROM information_schema.columns c \
                 LEFT JOIN ( \
                   SELECT ku.column_name \
                   FROM information_schema.table_constraints tc \
                   JOIN information_schema.key_column_usage ku \
                     ON tc.constraint_name = ku.constraint_name \
                   WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_name = $1 \
                 ) pk ON c.column_name = pk.column_name \
                 WHERE c.table_name = $1 \
                 ORDER BY c.ordinal_position",
                &[&table_name],
            )
            .await
            .map_err(|e| rust_ef::error::EFError::Query(format!("Column query error: {}", e)))?;

        let mut columns = Vec::new();
        for col_row in &col_rows {
            let col_name: String = col_row.get(0);
            let data_type: String = col_row.get(1);
            let is_nullable_str: String = col_row.get(2);
            let max_length: Option<i32> = col_row.get(3);
            let is_pk: bool = col_row.get(4);

            columns.push(DbColumn {
                name: col_name,
                data_type,
                is_nullable: is_nullable_str == "YES",
                is_primary_key: is_pk,
                max_length: max_length.map(|n| n as usize),
            });
        }

        tables.push(DbTable {
            name: table_name,
            columns,
        });
    }

    Ok(tables)
}
