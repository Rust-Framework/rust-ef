//! MySQL error mapping (EF-ERR-01).
//!
//! `sqlx` normalizes the vendor error into [`sqlx_core::Error::Database`], but
//! `Display` only prints `error returned from database: ...`. Mapping keeps the
//! structure — SQLSTATE, MySQL number, message and the failing SQL — in
//! [`EFError::Database`] so a production log identifies the failed statement.
//!
//! # Classification
//!
//! `sqlx` 0.9 enumerates the non-database failure modes as variants, so the
//! mapping is exhaustive on structure instead of on message text. The provider
//! maps:
//!
//! | `sqlx_core::Error` variant | Meaning | `EFError` |
//! |---|---|---|
//! | `Database` (unique/primary violation) | SQLSTATE kind `UniqueViolation` | `UniqueViolation` |
//! | `Database` (any other) | `DatabaseError` payload | `Database` |
//! | `Io`, `Tls` | transport / TLS handshake | `Connection` |
//! | `PoolTimedOut`, `PoolClosed`, `WorkerCrashed` | pool exhausted/closed, driver worker gone | `Connection` |
//! | `ColumnDecode`, `Decode`, `Encode`, `TypeNotFound` | value/type encoding or decoding | `TypeConversion` |
//! | `ColumnIndexOutOfBounds`, `ColumnNotFound` | projection shape (wrong column list) | `Query` |
//! | `InvalidArgument` | invalid argument to the driver call (statement construction) | `Query` |
//! | `RowNotFound` | query expected at least one row | `NotFound` |
//! | `Configuration`, `ConfigFile` | connection-string/option parsing | `Configuration` |
//! | `InvalidSavePointStatement`, `BeginFailed` | transaction protocol state | `Transaction` |
//! | `Protocol` | unexpected/invalid wire data — the connection cannot be trusted | `Connection` |
//! | `AnyDriverError`, unknown future variants | driver mapping bug | `Provider` |

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

    let detail = format!("{context}: {e}");
    let mapped = match &e {
        // Transport: socket I/O, TLS handshake, corrupted/unexpected wire data,
        // or a pool/driver worker that cannot hand out a live connection.
        SqlxError::Io(_)
        | SqlxError::Tls(_)
        | SqlxError::Protocol(_)
        | SqlxError::PoolTimedOut
        | SqlxError::PoolClosed
        | SqlxError::WorkerCrashed => EFError::connection(detail),

        // Value/type encoding and decoding.
        SqlxError::ColumnDecode { .. }
        | SqlxError::Decode(_)
        | SqlxError::Encode(_)
        | SqlxError::TypeNotFound { .. } => EFError::type_conversion(detail),

        // Projection/parameter shape: the statement's column list or argument
        // list does not match what the caller asked for.
        SqlxError::ColumnIndexOutOfBounds { .. }
        | SqlxError::ColumnNotFound(_)
        | SqlxError::InvalidArgument(_) => EFError::query(detail),

        SqlxError::RowNotFound => EFError::not_found(detail),

        SqlxError::Configuration(_) | SqlxError::ConfigFile(_) => EFError::configuration(detail),

        SqlxError::InvalidSavePointStatement | SqlxError::BeginFailed => {
            EFError::transaction(detail)
        }

        // `AnyDriverError` is a driver-mapping bug: provider-level, never
        // `Query` (which a web adapter maps to client data).
        SqlxError::AnyDriverError(_) => EFError::provider(detail),

        // Non-exhaustive enum: unknown future variants stay provider-level.
        _ => EFError::provider(detail),
    };
    mapped.with_source(Box::new(e))
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
    use super::*;
    use std::error::Error as _;
    use std::io;

    fn decode_error() -> SqlxError {
        SqlxError::Decode(Box::new(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad value",
        )))
    }

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

    #[test]
    fn transport_and_pool_failures_are_connections() {
        let cases = [
            SqlxError::Io(io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
            SqlxError::Tls(Box::new(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "handshake aborted",
            ))),
            SqlxError::Protocol("unexpected packet".to_string()),
            SqlxError::PoolTimedOut,
            SqlxError::PoolClosed,
            SqlxError::WorkerCrashed,
        ];
        for error in cases {
            let mapped = map_sqlx_error(error, "Execution error", "SELECT 1");
            assert!(
                matches!(mapped, EFError::Connection(..)),
                "expected Connection, got {mapped:?}"
            );
            assert!(
                mapped.source().is_some(),
                "the driver error must stay in the source chain"
            );
        }
    }

    #[test]
    fn encoding_and_decoding_failures_are_type_conversions() {
        let cases = [
            decode_error(),
            SqlxError::ColumnDecode {
                index: "0".to_string(),
                source: Box::new(io::Error::new(io::ErrorKind::InvalidData, "bad column")),
            },
            SqlxError::Encode(Box::new(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot encode",
            ))),
            SqlxError::TypeNotFound {
                type_name: "unknown_type".to_string(),
            },
        ];
        for error in cases {
            let mapped = map_sqlx_error(error, "Query error", "SELECT 1");
            assert!(
                matches!(mapped, EFError::TypeConversion(..)),
                "expected TypeConversion, got {mapped:?}"
            );
        }
    }

    #[test]
    fn projection_shape_failures_are_queries() {
        let cases = [
            SqlxError::ColumnIndexOutOfBounds { index: 3, len: 1 },
            SqlxError::ColumnNotFound("missing".to_string()),
            SqlxError::InvalidArgument("bad argument".to_string()),
        ];
        for error in cases {
            let mapped = map_sqlx_error(error, "Query error", "SELECT 1");
            assert!(
                matches!(mapped, EFError::Query(..)),
                "expected Query, got {mapped:?}"
            );
        }
    }

    #[test]
    fn row_not_found_and_configuration_stay_typed() {
        assert!(matches!(
            map_sqlx_error(SqlxError::RowNotFound, "Query error", "SELECT 1"),
            EFError::NotFound(..)
        ));
        assert!(matches!(
            map_sqlx_error(
                SqlxError::Configuration(Box::new(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "bad url",
                ))),
                "Connect",
                ""
            ),
            EFError::Configuration(..)
        ));
    }

    #[test]
    fn transaction_protocol_failures_are_transactions() {
        for error in [SqlxError::InvalidSavePointStatement, SqlxError::BeginFailed] {
            let mapped = map_sqlx_error(error, "Transaction error", "BEGIN");
            assert!(
                matches!(mapped, EFError::Transaction(..)),
                "expected Transaction, got {mapped:?}"
            );
        }
    }

    #[test]
    fn driver_mapping_bugs_are_provider_level_not_client_data() {
        let mapped = map_sqlx_error(
            SqlxError::AnyDriverError(Box::new(io::Error::new(
                io::ErrorKind::InvalidData,
                "unmapped driver error",
            ))),
            "Query error",
            "SELECT 1",
        );
        assert!(
            matches!(mapped, EFError::Provider(..)),
            "expected Provider, got {mapped:?}"
        );
    }
}
