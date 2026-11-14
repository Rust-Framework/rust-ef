//! MySQL error mapping (EF-ERR-01).
//!
//! `sqlx` normalizes the vendor error into [`sqlx_core::Error::Database`], but
//! `Display` only prints `error returned from database: ...`. Mapping keeps the
//! structure — SQLSTATE, MySQL number, message and the failing SQL — in
//! [`EFError::Database`] so a production log identifies the failed statement.

use rust_ef::error::{DatabaseErrorInfo, EFError};
use sqlx_core::error::Error as SqlxError;
use sqlx_mysql::MySqlDatabaseError;

/// Maps a `sqlx` error to a typed [`EFError`].
///
/// Duplicate-key errors (`ER_DUP_ENTRY` / `ER_DUP_KEY` / `ER_DUP_UNIQUE`) are
/// promoted to [`EFError::UniqueViolation`]; `sqlx` normalizes them to
/// [`sqlx_core::error::ErrorKind::UniqueViolation`]. The table and index name
/// are parsed from the driver message (`Duplicate entry '…' for key
/// 'table.index'`), which is the only place MySQL exposes them through `sqlx`.
pub(crate) fn map_sqlx_error(e: SqlxError, context: &str, sql: &str) -> EFError {
    if let SqlxError::Database(db) = &e {
        if db.is_unique_violation() {
            let (table, constraint) = parse_duplicate_key(db.message());
            return EFError::UniqueViolation { constraint, table };
        }

        let mut info = DatabaseErrorInfo::new(db.message());
        if let Some(sqlstate) = db.code() {
            info = info.with_sqlstate(sqlstate.into_owned());
        }
        // MySQL exposes the vendor number (e.g. 1059 "identifier too long")
        // separately from the SQLSTATE class.
        if let Some(mysql) = db.try_downcast_ref::<MySqlDatabaseError>() {
            info = info.with_code(mysql.number().to_string());
        }
        if let Some(table) = db.table() {
            info = info.with_table(table);
        }
        if let Some(constraint) = db.constraint() {
            info = info.with_constraint(constraint);
        }
        return EFError::database(info.with_sql(sql)).with_source(Box::new(e));
    }

    EFError::query(format!("{context}: {e}")).with_source(Box::new(e))
}

/// Parses `(table, constraint)` from a MySQL duplicate-entry message.
///
/// `"Duplicate entry '1' for key 'order_record.ux_order_tenant'"` yields
/// `(Some("order_record"), Some("ux_order_tenant"))`. `ER_DUP_ENTRY` reports
/// only the key name in some versions, in which case the table is `None`.
pub(crate) fn parse_duplicate_key(message: &str) -> (Option<String>, Option<String>) {
    const MARKER: &str = "for key '";
    let Some(start) = message.find(MARKER) else {
        return (None, None);
    };
    let rest = &message[start + MARKER.len()..];
    let Some(end) = rest.find('\'') else {
        return (None, None);
    };
    let key = &rest[..end];
    match key.split_once('.') {
        Some((table, constraint)) if !table.is_empty() && !constraint.is_empty() => {
            (Some(table.to_string()), Some(constraint.to_string()))
        }
        _ => (None, Some(key.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_duplicate_key;

    #[test]
    fn parses_table_and_index_from_duplicate_entry() {
        let (table, constraint) = parse_duplicate_key(
            "Duplicate entry '1-acme' for key 'order_record.ux_order_tenant_request'",
        );
        assert_eq!(table.as_deref(), Some("order_record"));
        assert_eq!(constraint.as_deref(), Some("ux_order_tenant_request"));
    }

    #[test]
    fn parses_primary_key_duplicate_without_table() {
        let (table, constraint) = parse_duplicate_key("Duplicate entry '42' for key 'PRIMARY'");
        assert_eq!(table, None);
        assert_eq!(constraint.as_deref(), Some("PRIMARY"));
    }

    #[test]
    fn unrelated_message_yields_none() {
        assert_eq!(
            parse_duplicate_key("Table 'db.missing' doesn't exist"),
            (None, None)
        );
    }
}
