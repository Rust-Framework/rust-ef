//! Database introspection for MySQL via `information_schema` (EF-5).
//!
//! Returns the provider-neutral [`DbTable`] model. MySQL can report columns,
//! indexes, foreign keys and `CHECK` constraints through `information_schema`;
//! triggers are returned as `None` because the catalog exposes only the
//! trigger body, not a re-executable `CREATE TRIGGER` statement.

use rust_ef::error::{EFError, EFResult};
use rust_ef::schema::{DbCheck, DbColumn, DbForeignKey, DbIndex, DbIndexColumn, DbTable};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_mysql::MySqlPool;

/// Reads tables and their schema from a MySQL database.
#[deprecated(note = "use IDatabaseProvider::introspect instead")]
pub async fn introspect_mysql(connection_string: &str) -> EFResult<Vec<DbTable>> {
    let pool = MySqlPool::connect(connection_string)
        .await
        .map_err(|e| EFError::connection(format!("MySQL connect failed: {e}")))?;
    introspect_mysql_pool(&pool).await
}

/// Introspects through an existing pool (used by the provider trait impl).
pub(crate) async fn introspect_mysql_pool(pool: &MySqlPool) -> EFResult<Vec<DbTable>> {
    let table_rows = query(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' \
         AND table_name NOT LIKE '__ef_%' \
         ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| EFError::query(format!("Table query error: {e}")))?;

    let mut tables = Vec::new();
    for table_row in table_rows {
        let table_name: String = table_row.try_get(0).map_err(map_row_err)?;
        let columns = columns(pool, &table_name).await?;
        let indexes = indexes(pool, &table_name).await?;
        let foreign_keys = foreign_keys(pool, &table_name).await?;
        let checks = checks(pool, &table_name).await?;
        tables.push(DbTable {
            name: table_name,
            columns,
            indexes,
            foreign_keys,
            checks,
            triggers: None,
        });
    }
    Ok(tables)
}

async fn columns(pool: &MySqlPool, table: &str) -> EFResult<Vec<DbColumn>> {
    // `COLUMN_TYPE` is the full type (`varchar(64)`, `tinyint(1)`,
    // `decimal(10,2)`, `datetime(6)`) whereas `DATA_TYPE` drops the length/
    // precision. Comparing the full spelling is what makes a freshly bootstrapped
    // MySQL schema diff to zero ALTERs (EF-DIFF-01).
    let col_rows = query(
        "SELECT c.column_name, c.column_type, c.is_nullable, \
                CAST(c.character_maximum_length AS SIGNED), \
                CAST(c.numeric_precision AS SIGNED), \
                CAST(c.numeric_scale AS SIGNED), \
                c.column_default, c.extra, \
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
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| EFError::query(format!("Column query error: {e}")))?;

    let mut columns = Vec::with_capacity(col_rows.len());
    for col_row in col_rows {
        let name: String = col_row.try_get(0).map_err(map_row_err)?;
        let data_type: String = col_row.try_get(1).map_err(map_row_err)?;
        let is_nullable: String = col_row.try_get(2).map_err(map_row_err)?;
        let max_length: Option<i64> = col_row.try_get(3).map_err(map_row_err)?;
        let precision: Option<i64> = col_row.try_get(4).map_err(map_row_err)?;
        let scale: Option<i64> = col_row.try_get(5).map_err(map_row_err)?;
        let default_value: Option<String> = col_row.try_get(6).map_err(map_row_err)?;
        let extra: String = col_row.try_get(7).map_err(map_row_err)?;
        let is_pk: i32 = col_row.try_get(8).map_err(map_row_err)?;

        columns.push(DbColumn {
            name,
            data_type,
            is_nullable: is_nullable == "YES",
            is_primary_key: is_pk != 0,
            is_auto_increment: extra.to_ascii_lowercase().contains("auto_increment"),
            max_length: max_length.map(|n| n as usize),
            precision: precision.map(|n| n as u32),
            scale: scale.map(|n| n as u32),
            default_value,
        });
    }
    Ok(columns)
}

/// One raw `information_schema.statistics` row used while grouping key parts.
struct RawIndexRow {
    name: String,
    non_unique: i64,
    column: Option<String>,
    expression: Option<String>,
}

async fn indexes(pool: &MySqlPool, table: &str) -> EFResult<Vec<DbIndex>> {
    // MySQL 8.0.13+ exposes functional key parts through `EXPRESSION`
    // (`COLUMN_NAME` is NULL for them); older servers reject that column, so
    // fall back to the column-only query. Without the fallback the whole
    // introspection fails on MySQL 5.7/8.0.12.
    let rows = match query(
        "SELECT index_name, CAST(non_unique AS SIGNED), column_name, expression \
         FROM information_schema.statistics \
         WHERE table_schema = DATABASE() AND table_name = ? \
         ORDER BY index_name, seq_in_index",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|row| {
                Ok(RawIndexRow {
                    name: row.try_get(0).map_err(map_row_err)?,
                    non_unique: row.try_get(1).map_err(map_row_err)?,
                    column: row.try_get(2).map_err(map_row_err)?,
                    expression: row.try_get(3).map_err(map_row_err)?,
                })
            })
            .collect::<EFResult<Vec<_>>>()?,
        Err(error) if is_unknown_column(&error) => {
            let rows = query(
                "SELECT index_name, CAST(non_unique AS SIGNED), column_name \
                 FROM information_schema.statistics \
                 WHERE table_schema = DATABASE() AND table_name = ? \
                 ORDER BY index_name, seq_in_index",
            )
            .bind(table)
            .fetch_all(pool)
            .await
            .map_err(|e| EFError::query(format!("Index query error: {e}")))?;
            rows.into_iter()
                .map(|row| {
                    Ok(RawIndexRow {
                        name: row.try_get(0).map_err(map_row_err)?,
                        non_unique: row.try_get(1).map_err(map_row_err)?,
                        column: row.try_get(2).map_err(map_row_err)?,
                        expression: None,
                    })
                })
                .collect::<EFResult<Vec<_>>>()?
        }
        Err(error) => return Err(EFError::query(format!("Index query error: {error}"))),
    };

    let mut indexes: Vec<DbIndex> = Vec::new();
    for row in rows {
        if row.name == "PRIMARY" {
            // The primary key is represented by `DbColumn::is_primary_key`.
            continue;
        }
        let part = match (row.column, row.expression) {
            (Some(column), _) => Some(DbIndexColumn {
                value: column,
                is_expression: false,
            }),
            (None, Some(expression)) => Some(DbIndexColumn {
                value: expression,
                is_expression: true,
            }),
            (None, None) => None,
        };
        match indexes.last_mut() {
            Some(index) if index.name == row.name => {
                if let Some(part) = part {
                    index.columns.push(part);
                }
            }
            _ => indexes.push(DbIndex {
                name: row.name,
                columns: part.into_iter().collect(),
                is_unique: row.non_unique == 0,
                // MySQL has no partial indexes (the framework emulates them
                // with functional indexes on MySQL 8).
                filter: None,
            }),
        }
    }
    Ok(indexes)
}

async fn foreign_keys(pool: &MySqlPool, table: &str) -> EFResult<Vec<DbForeignKey>> {
    let rows = query(
        "SELECT k.constraint_name, k.column_name, k.referenced_table_name, \
                k.referenced_column_name, r.delete_rule, r.update_rule \
         FROM information_schema.key_column_usage k \
         JOIN information_schema.referential_constraints r \
           ON r.constraint_schema = k.constraint_schema \
          AND r.constraint_name = k.constraint_name \
          AND r.table_name = k.table_name \
         WHERE k.table_schema = DATABASE() AND k.table_name = ? \
           AND k.referenced_table_name IS NOT NULL \
         ORDER BY k.constraint_name, k.ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| EFError::query(format!("Foreign key query error: {e}")))?;

    let mut foreign_keys: Vec<DbForeignKey> = Vec::new();
    for row in rows {
        let name: String = row.try_get(0).map_err(map_row_err)?;
        let column: String = row.try_get(1).map_err(map_row_err)?;
        let referenced_table: String = row.try_get(2).map_err(map_row_err)?;
        let referenced_column: String = row.try_get(3).map_err(map_row_err)?;
        let on_delete: String = row.try_get(4).map_err(map_row_err)?;
        let on_update: String = row.try_get(5).map_err(map_row_err)?;
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
                on_delete: normalize_action(&on_delete),
                on_update: normalize_action(&on_update),
            }),
        }
    }
    Ok(foreign_keys)
}

/// MySQL 8 exposes `CHECK` constraints through `information_schema`.
///
/// Older servers have no `check_constraints` table; in that case the result is
/// `None` ("not introspected") rather than an empty list.
async fn checks(pool: &MySqlPool, table: &str) -> EFResult<Option<Vec<DbCheck>>> {
    let rows = match query(
        "SELECT tc.constraint_name, cc.check_clause \
         FROM information_schema.table_constraints tc \
         JOIN information_schema.check_constraints cc \
           ON cc.constraint_schema = tc.constraint_schema \
          AND cc.constraint_name = tc.constraint_name \
         WHERE tc.table_schema = DATABASE() AND tc.table_name = ? \
           AND tc.constraint_type = 'CHECK' \
         ORDER BY tc.constraint_name",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) if is_missing_catalog(&error) => return Ok(None),
        Err(error) => return Err(EFError::query(format!("Check query error: {error}"))),
    };

    let mut checks = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.try_get(0).map_err(map_row_err)?;
        let expression: String = row.try_get(1).map_err(map_row_err)?;
        checks.push(DbCheck {
            name: Some(name),
            expression,
        });
    }
    Ok(Some(checks))
}

/// `NO ACTION` is the database default and is represented as `None`.
fn normalize_action(action: &str) -> Option<String> {
    let action = action.trim();
    if action.is_empty() || action.eq_ignore_ascii_case("NO ACTION") {
        None
    } else {
        Some(action.to_string())
    }
}

/// SQLSTATE `42S02`: unknown table — e.g. `check_constraints` on MySQL < 8.0.16.
fn is_missing_catalog(error: &sqlx_core::Error) -> bool {
    matches!(
        error,
        sqlx_core::Error::Database(database_error)
            if database_error.code().as_deref() == Some("42S02")
    )
}

/// SQLSTATE `42S22`: unknown column — e.g. `statistics.EXPRESSION` on
/// MySQL < 8.0.13.
fn is_unknown_column(error: &sqlx_core::Error) -> bool {
    matches!(
        error,
        sqlx_core::Error::Database(database_error)
            if database_error.code().as_deref() == Some("42S22")
    )
}

fn map_row_err(e: sqlx_core::Error) -> EFError {
    EFError::query(format!("Row read error: {e}"))
}
