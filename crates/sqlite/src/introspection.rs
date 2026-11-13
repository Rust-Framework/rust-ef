//! Live-schema introspection for SQLite (EF-5).
//!
//! SQLite exposes its schema through `sqlite_master` (table/index/trigger DDL)
//! and the `pragma_table_info` / `pragma_index_list` / `pragma_index_info` /
//! `pragma_foreign_key_list` table-valued functions. Constraint and index names
//! are not part of the PRAGMA output, so they are parsed from
//! `sqlite_master.sql` where possible and left `None` otherwise.
//!
//! The result is dialect-neutral ([`rust_ef::schema::DbTable`]), so callers can
//! diff it against a model without dispatching on the provider.

use rusqlite::{Connection, OptionalExtension};
use rust_ef::error::{EFError, EFResult};
use rust_ef::schema::{
    DbCheck, DbColumn, DbForeignKey, DbIndex, DbIndexColumn, DbTable, DbTrigger,
};

/// Reads the live schema of a SQLite database from its path.
///
/// Migration-internal tables (`__ef_migrations_history` etc.) and SQLite
/// internal tables (`sqlite_%`) are excluded.
#[deprecated(note = "use IDatabaseProvider::introspect instead")]
pub async fn introspect_sqlite(connection_string: &str) -> EFResult<Vec<DbTable>> {
    let path = connection_string.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = Connection::open(&path)
            .map_err(|e| EFError::connection(format!("SQLite open failed: {}", e)))?;
        introspect_connection(&conn)
    })
    .await
    .map_err(|e| EFError::query(format!("SQLite introspection task failed: {}", e)))?
}

fn introspection_error(context: &str, error: rusqlite::Error) -> EFError {
    EFError::query(format!("SQLite introspection {context}: {error}"))
}

/// Reads the live schema from an open connection (synchronous).
///
/// Used by the pooled and in-memory providers, which already hold a connection.
pub(crate) fn introspect_connection(conn: &Connection) -> EFResult<Vec<DbTable>> {
    let mut tables = Vec::new();
    for (table_name, create_sql) in table_definitions(conn)? {
        let columns = columns(conn, &table_name, create_sql.as_deref())?;
        let indexes = indexes(conn, &table_name)?;
        let foreign_keys = foreign_keys(conn, &table_name)?;
        let checks = parse_checks(create_sql.as_deref());
        let triggers = triggers(conn, &table_name)?;
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

fn table_definitions(conn: &Connection) -> EFResult<Vec<(String, Option<String>)>> {
    let mut stmt = conn
        .prepare(
            "SELECT name, sql FROM sqlite_master \
             WHERE type = 'table' \
               AND name NOT LIKE 'sqlite_%' \
               AND name NOT LIKE '__ef_%' \
             ORDER BY name",
        )
        .map_err(|e| introspection_error("table query", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| introspection_error("table query", e))?;
    let mut tables = Vec::new();
    for row in rows {
        tables.push(row.map_err(|e| introspection_error("table read", e))?);
    }
    Ok(tables)
}

fn columns(conn: &Connection, table: &str, create_sql: Option<&str>) -> EFResult<Vec<DbColumn>> {
    let mut stmt = conn
        .prepare(
            "SELECT name, type, \"notnull\", pk, dflt_value \
             FROM pragma_table_info(?1) ORDER BY cid",
        )
        .map_err(|e| introspection_error("column query", e))?;
    #[allow(clippy::type_complexity)]
    let rows = stmt
        .query_map([table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| introspection_error("column query", e))?;
    let mut raw = Vec::new();
    for row in rows {
        raw.push(row.map_err(|e| introspection_error("column read", e))?);
    }
    let primary_key_count = raw.iter().filter(|(_, _, _, pk, _)| *pk > 0).count();
    // `INTEGER PRIMARY KEY` is a rowid alias (and therefore auto-generated)
    // only for ordinary rowid tables with a single-column primary key.
    let without_rowid = create_sql
        .map(|sql| sql.to_ascii_uppercase().contains("WITHOUT ROWID"))
        .unwrap_or(false);

    let mut columns = Vec::with_capacity(raw.len());
    for (name, data_type, notnull, pk, default_value) in raw {
        let is_primary_key = pk > 0;
        let is_auto_increment = !without_rowid
            && is_primary_key
            && primary_key_count == 1
            && data_type.eq_ignore_ascii_case("INTEGER");
        let (max_length, precision, scale) = parse_type_metadata(&data_type);
        columns.push(DbColumn {
            name,
            data_type,
            is_nullable: notnull == 0 && !is_primary_key,
            is_primary_key,
            is_auto_increment,
            max_length,
            precision,
            scale,
            default_value,
        });
    }
    Ok(columns)
}

/// Splits a declared SQLite type into length / precision / scale metadata.
///
/// SQLite type names are free-form; only the conventional
/// `VARCHAR(n)` / `CHAR(n)` and `DECIMAL(p, s)` / `NUMERIC(p, s)` shapes are
/// recognised. Everything else reports `None` for all three values.
fn parse_type_metadata(data_type: &str) -> (Option<usize>, Option<u32>, Option<u32>) {
    let Some(open) = data_type.find('(') else {
        return (None, None, None);
    };
    let Some(close) = data_type[open..].find(')') else {
        return (None, None, None);
    };
    let inner = &data_type[open + 1..open + close];
    let base = data_type[..open].trim().to_ascii_uppercase();
    let mut parts = inner.split(',').map(|part| part.trim().parse::<u32>());
    let first = parts.next().and_then(Result::ok);
    let second = parts.next().and_then(Result::ok);
    if base.contains("CHAR") {
        (first.map(|n| n as usize), None, None)
    } else if base.contains("DEC") || base.contains("NUMERIC") {
        (None, first, second)
    } else {
        (None, None, None)
    }
}

fn indexes(conn: &Connection, table: &str) -> EFResult<Vec<DbIndex>> {
    let mut stmt = conn
        .prepare("SELECT name, \"unique\", origin FROM pragma_index_list(?1) ORDER BY name")
        .map_err(|e| introspection_error("index query", e))?;
    let rows = stmt
        .query_map([table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| introspection_error("index query", e))?;
    let mut indexes = Vec::new();
    for row in rows {
        let (name, unique, origin) = row.map_err(|e| introspection_error("index read", e))?;
        // `origin = "c"` means an explicit `CREATE INDEX`; its name matches the
        // model-level index name. `"u"`/`"pk"` are implicit unique/primary-key
        // indexes whose names SQLite generates (`sqlite_autoindex_*`), so they
        // are represented through column flags instead.
        if origin != "c" {
            continue;
        }
        let (columns, filter) = index_parts(conn, &name)?;
        indexes.push(DbIndex {
            name,
            columns,
            is_unique: unique != 0,
            filter,
        });
    }
    Ok(indexes)
}

fn index_parts(
    conn: &Connection,
    index_name: &str,
) -> EFResult<(Vec<DbIndexColumn>, Option<String>)> {
    let sql: Option<String> = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index_name],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| introspection_error("index sql query", e))?;
    let parsed = sql.as_deref().and_then(parse_create_index);

    let mut stmt = conn
        .prepare("SELECT seqno, cid, name FROM pragma_index_info(?1) ORDER BY seqno")
        .map_err(|e| introspection_error("index column query", e))?;
    let rows = stmt
        .query_map([index_name], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| introspection_error("index column query", e))?;

    let mut columns = Vec::new();
    for row in rows {
        let (seqno, cid, name) = row.map_err(|e| introspection_error("index column read", e))?;
        match name {
            Some(name) => columns.push(DbIndexColumn {
                value: name,
                is_expression: false,
            }),
            // `cid = -2` is a functional/expression key part; SQLite does not
            // return its text through PRAGMA, so recover it from the original
            // `CREATE INDEX` statement when parsing succeeded.
            None if cid == -2 => {
                if let Some(part) = parsed
                    .as_ref()
                    .and_then(|parsed| parsed.columns.get(seqno as usize))
                {
                    columns.push(DbIndexColumn {
                        value: part.clone(),
                        is_expression: true,
                    });
                }
            }
            None => {}
        }
    }
    let filter = parsed.and_then(|parsed| parsed.filter);
    Ok((columns, filter))
}

struct ParsedIndex {
    columns: Vec<String>,
    filter: Option<String>,
}

fn parse_create_index(sql: &str) -> Option<ParsedIndex> {
    let lower = sql.to_ascii_lowercase();
    let on = lower.find(" on ")?;
    let open = sql[on + 4..].find('(')? + on + 4;
    let close = matching_paren(sql.as_bytes(), open)?;
    let columns = split_top_level(&sql[open + 1..close]);
    let rest = sql[close + 1..].trim();
    let filter = if rest.is_empty() {
        None
    } else {
        let lower_rest = rest.to_ascii_lowercase();
        lower_rest.find("where ").map(|i| {
            rest[i + "where ".len()..]
                .trim()
                .trim_end_matches(';')
                .trim()
                .to_string()
        })
    };
    Some(ParsedIndex { columns, filter })
}

fn foreign_keys(conn: &Connection, table: &str) -> EFResult<Vec<DbForeignKey>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, \"table\", \"from\", \"to\", on_delete, on_update \
             FROM pragma_foreign_key_list(?1) ORDER BY id, seq",
        )
        .map_err(|e| introspection_error("foreign key query", e))?;
    #[allow(clippy::type_complexity)]
    let rows = stmt
        .query_map([table], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| introspection_error("foreign key query", e))?;

    let mut grouped: Vec<(i64, DbForeignKey)> = Vec::new();
    for row in rows {
        let (id, referenced_table, from, to, on_delete, on_update) =
            row.map_err(|e| introspection_error("foreign key read", e))?;
        if let Some((last_id, foreign_key)) = grouped.last_mut() {
            if *last_id == id {
                foreign_key.columns.push(from);
                if let Some(to) = to {
                    foreign_key.referenced_columns.push(to);
                }
                continue;
            }
        }
        // SQLite omits the referenced column when the FK targets the target
        // table's primary key, so resolve it from the target table.
        let referenced_columns = match to {
            Some(to) => vec![to],
            None => primary_key_columns(conn, &referenced_table)?,
        };
        grouped.push((
            id,
            DbForeignKey {
                name: None,
                columns: vec![from],
                referenced_table,
                referenced_columns,
                on_delete: normalize_fk_action(&on_delete),
                on_update: normalize_fk_action(&on_update),
            },
        ));
    }
    Ok(grouped
        .into_iter()
        .map(|(_, foreign_key)| foreign_key)
        .collect())
}

fn primary_key_columns(conn: &Connection, table: &str) -> EFResult<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT name FROM pragma_table_info(?1) WHERE pk > 0 ORDER BY pk")
        .map_err(|e| introspection_error("primary key query", e))?;
    let rows = stmt
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(|e| introspection_error("primary key query", e))?;
    let mut columns = Vec::new();
    for row in rows {
        columns.push(row.map_err(|e| introspection_error("primary key read", e))?);
    }
    Ok(columns)
}

/// `NO ACTION` is the database default and is represented as `None`.
fn normalize_fk_action(action: &str) -> Option<String> {
    let action = action.trim();
    if action.is_empty() || action.eq_ignore_ascii_case("NO ACTION") {
        None
    } else {
        Some(action.to_string())
    }
}

fn triggers(conn: &Connection, table: &str) -> EFResult<Option<Vec<DbTrigger>>> {
    let mut stmt = conn
        .prepare(
            "SELECT name, sql FROM sqlite_master \
             WHERE type = 'trigger' AND tbl_name = ?1 ORDER BY name",
        )
        .map_err(|e| introspection_error("trigger query", e))?;
    let rows = stmt
        .query_map([table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| introspection_error("trigger query", e))?;
    let mut triggers = Vec::new();
    for row in rows {
        let (name, sql) = row.map_err(|e| introspection_error("trigger read", e))?;
        triggers.push(DbTrigger {
            name,
            sql: sql.unwrap_or_default(),
        });
    }
    // SQLite always exposes triggers through `sqlite_master`, so an empty list
    // is a confirmed "no triggers", not "unknown".
    Ok(Some(triggers))
}

/// Parses table-level `CHECK (...)` constraints from `CREATE TABLE` SQL.
///
/// Returns:
/// - `Some(vec![])` — the table has no `CHECK` constraints;
/// - `Some(checks)` — every `CHECK` was parsed (names are `None`: SQLite does
///   not expose constraint names through PRAGMA, and structural matching is
///   used by the migration diff);
/// - `None` — a `CHECK` keyword was found but its expression could not be
///   parsed reliably, so the whole list is unknown.
fn parse_checks(create_sql: Option<&str>) -> Option<Vec<DbCheck>> {
    let sql = create_sql?;
    let bytes = sql.as_bytes();
    let mut checks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                i = skip_quoted(bytes, i);
            }
            b'C' | b'c' => {
                if starts_with_ascii_case_insensitive(&sql[i..], "CHECK")
                    && is_word_boundary(bytes, i, "CHECK".len())
                {
                    let mut j = i + "CHECK".len();
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'(' {
                        let close = matching_paren(bytes, j)?;
                        checks.push(DbCheck {
                            name: None,
                            expression: sql[j + 1..close].trim().to_string(),
                        });
                        i = close + 1;
                        continue;
                    }
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    Some(checks)
}

fn starts_with_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack.len() >= needle.len()
        && haystack.as_bytes()[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
}

fn is_word_boundary(bytes: &[u8], start: usize, len: usize) -> bool {
    let before_ok = start == 0 || !is_identifier_byte(bytes[start - 1]);
    let after = start + len;
    let after_ok = after >= bytes.len() || !is_identifier_byte(bytes[after]);
    before_ok && after_ok
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn skip_quoted(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            // Doubled quotes are escapes inside the same literal.
            if i + 1 < bytes.len() && bytes[i + 1] == quote {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    i
}

/// Returns the index of the `)` matching the `(` at `open`.
fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                i = skip_quoted(bytes, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Splits a comma-separated list at parenthesis depth 0.
fn split_top_level(inner: &str) -> Vec<String> {
    let bytes = inner.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                i = skip_quoted(bytes, i);
                continue;
            }
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(inner[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(inner[start..].trim().to_string());
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_type_metadata_for_char_and_decimal_types() {
        assert_eq!(parse_type_metadata("VARCHAR(120)"), (Some(120), None, None));
        assert_eq!(
            parse_type_metadata("DECIMAL(12, 2)"),
            (None, Some(12), Some(2))
        );
        assert_eq!(parse_type_metadata("INTEGER"), (None, None, None));
        assert_eq!(parse_type_metadata("TEXT"), (None, None, None));
    }

    #[test]
    fn parses_check_constraints_from_create_table_sql() {
        let sql = "CREATE TABLE t (\
            id INTEGER PRIMARY KEY, \
            price INTEGER NOT NULL, \
            CHECK (price > 0), \
            CONSTRAINT ck_qty CHECK (quantity >= 1)\
        )";
        let checks = parse_checks(Some(sql)).expect("checks introspected");
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].expression, "price > 0");
        assert_eq!(checks[1].expression, "quantity >= 1");
        assert_eq!(checks[0].name, None);
    }

    #[test]
    fn check_keywords_inside_string_literals_are_ignored() {
        let sql = "CREATE TABLE t (note TEXT DEFAULT 'CHECK (fake)')";
        let checks = parse_checks(Some(sql)).expect("checks introspected");
        assert!(checks.is_empty());
    }

    #[test]
    fn parses_partial_index_filter_and_expression_parts() {
        let parsed = parse_create_index(
            "CREATE UNIQUE INDEX ux_t_lower ON t (lower(name), tenant_id) WHERE status = 'ON'",
        )
        .expect("index parsed");
        assert_eq!(
            parsed.columns,
            vec!["lower(name)".to_string(), "tenant_id".to_string()]
        );
        assert_eq!(parsed.filter.as_deref(), Some("status = 'ON'"));
    }
}
