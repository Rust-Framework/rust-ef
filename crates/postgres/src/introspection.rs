//! Database introspection for PostgreSQL (EF-5).
//!
//! Returns the provider-neutral [`DbTable`] model. PostgreSQL exposes columns,
//! indexes, foreign keys, `CHECK` constraints and triggers through
//! `information_schema` and the system catalogs (`pg_index`, `pg_constraint`,
//! `pg_trigger`).

use crate::tls::PgTlsMode;
use rust_ef::error::{EFError, EFResult};
use rust_ef::schema::{
    DbCheck, DbColumn, DbForeignKey, DbIndex, DbIndexColumn, DbTable, DbTrigger,
};
use tokio_postgres::NoTls;

/// Reads all tables and their schema from a PostgreSQL database.
///
/// The `tls` parameter controls transport encryption. For local dev
/// (plaintext), pass [`PgTlsMode::Disable`]; for production, use
/// [`PgTlsMode::Require`] with a configured `TlsConnector`.
#[deprecated(note = "use IDatabaseProvider::introspect instead")]
pub async fn introspect_postgres(
    connection_string: &str,
    tls: PgTlsMode,
) -> EFResult<Vec<DbTable>> {
    let config: tokio_postgres::Config = connection_string
        .parse()
        .map_err(|e| EFError::connection(format!("Invalid connection string: {}", e)))?;

    let client = match tls {
        PgTlsMode::Disable => {
            let (client, connection) = config
                .connect(NoTls)
                .await
                .map_err(|e| EFError::connection(format!("Connection failed: {}", e)))?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("PostgreSQL connection error: {}", e);
                }
            });
            client
        }
        PgTlsMode::Require(connector) => {
            let tls_connector = postgres_native_tls::MakeTlsConnector::new(connector);
            let (client, connection) = config
                .connect(tls_connector)
                .await
                .map_err(|e| EFError::connection(format!("Connection failed: {}", e)))?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("PostgreSQL connection error: {}", e);
                }
            });
            client
        }
    };

    introspect_postgres_client(&client).await
}

/// Introspects through an existing client (used by the provider trait impl and
/// by [`introspect_postgres`]).
pub(crate) async fn introspect_postgres_client(
    client: &tokio_postgres::Client,
) -> EFResult<Vec<DbTable>> {
    let table_rows = client
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE' \
             AND table_name NOT LIKE '__ef_%' \
             ORDER BY table_name",
            &[],
        )
        .await
        .map_err(|e| EFError::query(format!("Table query error: {}", e)))?;

    let mut tables = Vec::new();
    for table_row in &table_rows {
        let table_name: String = table_row.get(0);
        let columns = columns(client, &table_name).await?;
        let indexes = indexes(client, &table_name).await?;
        let foreign_keys = foreign_keys(client, &table_name).await?;
        let checks = checks(client, &table_name).await?;
        let triggers = triggers(client, &table_name).await?;
        tables.push(DbTable {
            name: table_name,
            columns,
            indexes,
            foreign_keys,
            checks,
            triggers,
        });
    }
    Ok(tables)
}

async fn columns(client: &tokio_postgres::Client, table: &str) -> EFResult<Vec<DbColumn>> {
    let col_rows = client
        .query(
            "SELECT c.column_name, c.data_type, c.is_nullable, c.character_maximum_length, \
                    c.numeric_precision, c.numeric_scale, c.column_default, c.is_identity, \
                    CASE WHEN pk.column_name IS NOT NULL THEN true ELSE false END AS is_pk \
             FROM information_schema.columns c \
             LEFT JOIN ( \
               SELECT ku.column_name \
               FROM information_schema.table_constraints tc \
               JOIN information_schema.key_column_usage ku \
                 ON tc.constraint_name = ku.constraint_name \
                AND tc.table_schema = ku.table_schema \
               WHERE tc.constraint_type = 'PRIMARY KEY' \
                 AND tc.table_schema = 'public' AND tc.table_name = $1 \
             ) pk ON c.column_name = pk.column_name \
             WHERE c.table_schema = 'public' AND c.table_name = $1 \
             ORDER BY c.ordinal_position",
            &[&table],
        )
        .await
        .map_err(|e| EFError::query(format!("Column query error: {}", e)))?;

    let mut columns = Vec::with_capacity(col_rows.len());
    for col_row in &col_rows {
        let name: String = col_row.get(0);
        let data_type: String = col_row.get(1);
        let is_nullable: String = col_row.get(2);
        let max_length: Option<i32> = col_row.get(3);
        let precision: Option<i32> = col_row.get(4);
        let scale: Option<i32> = col_row.get(5);
        let default_value: Option<String> = col_row.get(6);
        let is_identity: String = col_row.get(7);
        let is_pk: bool = col_row.get(8);

        let default_sql = default_value.as_deref().unwrap_or_default();
        columns.push(DbColumn {
            name,
            data_type,
            is_nullable: is_nullable == "YES",
            is_primary_key: is_pk,
            is_auto_increment: is_identity == "YES" || default_sql.starts_with("nextval("),
            max_length: max_length.map(|n| n as usize),
            precision: precision.map(|n| n as u32),
            scale: scale.map(|n| n as u32),
            default_value,
        });
    }
    Ok(columns)
}

async fn indexes(client: &tokio_postgres::Client, table: &str) -> EFResult<Vec<DbIndex>> {
    let rows = client
        .query(
            "SELECT i.relname AS index_name, \
                    ix.indisunique AS is_unique, \
                    pg_get_indexdef(i.oid, k.ord::int, true) AS part, \
                    (ix.indkey[k.ord - 1] = 0) AS is_expression, \
                    pg_get_expr(ix.indpred, ix.indrelid) AS filter \
             FROM pg_class t \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             JOIN pg_index ix ON ix.indrelid = t.oid \
             JOIN pg_class i ON i.oid = ix.indexrelid \
             CROSS JOIN LATERAL generate_series(1, ix.indnkeyatts) AS k(ord) \
             WHERE n.nspname = 'public' AND t.relname = $1 AND NOT ix.indisprimary \
             ORDER BY i.relname, k.ord",
            &[&table],
        )
        .await
        .map_err(|e| EFError::query(format!("Index query error: {}", e)))?;

    let mut indexes: Vec<DbIndex> = Vec::new();
    for row in &rows {
        let name: String = row.get(0);
        let is_unique: bool = row.get(1);
        let part: String = row.get(2);
        let is_expression: bool = row.get(3);
        let filter: Option<String> = row.get(4);
        match indexes.last_mut() {
            Some(index) if index.name == name => {
                index.columns.push(DbIndexColumn {
                    value: part,
                    is_expression,
                });
            }
            _ => indexes.push(DbIndex {
                name,
                columns: vec![DbIndexColumn {
                    value: part,
                    is_expression,
                }],
                is_unique,
                filter,
            }),
        }
    }
    Ok(indexes)
}

async fn foreign_keys(client: &tokio_postgres::Client, table: &str) -> EFResult<Vec<DbForeignKey>> {
    let rows = client
        .query(
            "SELECT con.conname, a.attname AS column_name, \
                    rt.relname AS referenced_table, ra.attname AS referenced_column, \
                    con.confdeltype::text AS on_delete, con.confupdtype::text AS on_update \
             FROM pg_constraint con \
             JOIN pg_class t ON t.oid = con.conrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             JOIN pg_class rt ON rt.oid = con.confrelid \
             CROSS JOIN LATERAL unnest(con.conkey, con.confkey) \
               WITH ORDINALITY AS k(attnum, refattnum, ord) \
             JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum \
             JOIN pg_attribute ra ON ra.attrelid = rt.oid AND ra.attnum = k.refattnum \
             WHERE n.nspname = 'public' AND t.relname = $1 AND con.contype = 'f' \
             ORDER BY con.conname, k.ord",
            &[&table],
        )
        .await
        .map_err(|e| EFError::query(format!("Foreign key query error: {}", e)))?;

    let mut foreign_keys: Vec<DbForeignKey> = Vec::new();
    for row in &rows {
        let name: String = row.get(0);
        let column: String = row.get(1);
        let referenced_table: String = row.get(2);
        let referenced_column: String = row.get(3);
        let on_delete: String = row.get(4);
        let on_update: String = row.get(5);
        match foreign_keys.last_mut() {
            Some(foreign_key) if foreign_key.name.as_deref() == Some(name.as_str()) => {
                foreign_key.columns.push(column);
                foreign_key.referenced_columns.push(referenced_column);
            }
            _ => foreign_keys.push(DbForeignKey {
                name: Some(name),
                columns: vec![column],
                referenced_table,
                referenced_columns: vec![referenced_column],
                on_delete: fk_action(&on_delete),
                on_update: fk_action(&on_update),
            }),
        }
    }
    Ok(foreign_keys)
}

/// Maps PostgreSQL's one-letter FK action code to its SQL keyword.
///
/// `a` (`NO ACTION`, the default) is represented as `None`.
fn fk_action(code: &str) -> Option<String> {
    match code.trim() {
        "" | "a" => None,
        "r" => Some("RESTRICT".to_string()),
        "c" => Some("CASCADE".to_string()),
        "n" => Some("SET NULL".to_string()),
        "d" => Some("SET DEFAULT".to_string()),
        _ => Some(code.to_string()),
    }
}

async fn checks(client: &tokio_postgres::Client, table: &str) -> EFResult<Option<Vec<DbCheck>>> {
    let rows = client
        .query(
            "SELECT con.conname, pg_get_constraintdef(con.oid, true) \
             FROM pg_constraint con \
             JOIN pg_class t ON t.oid = con.conrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE n.nspname = 'public' AND t.relname = $1 AND con.contype = 'c' \
             ORDER BY con.conname",
            &[&table],
        )
        .await
        .map_err(|e| EFError::query(format!("Check query error: {}", e)))?;

    let mut checks = Vec::with_capacity(rows.len());
    for row in &rows {
        let name: String = row.get(0);
        let definition: String = row.get(1);
        checks.push(DbCheck {
            name: Some(name),
            expression: strip_check_wrapper(&definition),
        });
    }
    // `pg_constraint` always exposes CHECK constraints.
    Ok(Some(checks))
}

/// Turns `CHECK ((price > 0))` into `(price > 0)` (one wrapper layer removed,
/// matching how the model renders a `has_check` expression).
fn strip_check_wrapper(definition: &str) -> String {
    let trimmed = definition.trim();
    let rest = trimmed
        .strip_prefix("CHECK ")
        .or_else(|| trimmed.strip_prefix("check "))
        .unwrap_or(trimmed)
        .trim();
    if rest.len() >= 2 && rest.starts_with('(') && rest.ends_with(')') {
        rest[1..rest.len() - 1].trim().to_string()
    } else {
        rest.to_string()
    }
}

async fn triggers(
    client: &tokio_postgres::Client,
    table: &str,
) -> EFResult<Option<Vec<DbTrigger>>> {
    let rows = client
        .query(
            "SELECT tg.tgname, pg_get_triggerdef(tg.oid) \
             FROM pg_trigger tg \
             JOIN pg_class t ON t.oid = tg.tgrelid \
             JOIN pg_namespace n ON n.oid = t.relnamespace \
             WHERE n.nspname = 'public' AND t.relname = $1 AND NOT tg.tgisinternal \
             ORDER BY tg.tgname",
            &[&table],
        )
        .await
        .map_err(|e| EFError::query(format!("Trigger query error: {}", e)))?;

    let mut triggers = Vec::with_capacity(rows.len());
    for row in &rows {
        let name: String = row.get(0);
        let sql: String = row.get(1);
        triggers.push(DbTrigger { name, sql });
    }
    // `pg_trigger` always exposes triggers.
    Ok(Some(triggers))
}

#[cfg(test)]
mod tests {
    use super::{fk_action, strip_check_wrapper};

    #[test]
    fn maps_postgres_fk_action_codes() {
        assert_eq!(fk_action("a"), None);
        assert_eq!(fk_action("c").as_deref(), Some("CASCADE"));
        assert_eq!(fk_action("n").as_deref(), Some("SET NULL"));
        assert_eq!(fk_action("r").as_deref(), Some("RESTRICT"));
    }

    #[test]
    fn strips_check_wrapper() {
        assert_eq!(strip_check_wrapper("CHECK ((price > 0))"), "(price > 0)");
        assert_eq!(strip_check_wrapper("CHECK (price > 0)"), "price > 0");
    }
}
