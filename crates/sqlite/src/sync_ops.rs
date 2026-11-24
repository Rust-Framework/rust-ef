use rusqlite::ffi::ErrorCode;
use rust_ef::error::{DatabaseErrorInfo, EFError, EFResult};
use rust_ef::provider::DbValue;

/// Maps a `rusqlite` error to a typed [`EFError`].
///
/// Unique/primary-key violations are promoted to
/// [`EFError::UniqueViolation`] so callers no longer need to sniff the driver
/// message. SQLite reports the violated columns instead of a constraint name
/// (`UNIQUE constraint failed: order_record.tenant_id, ...`), so `constraint`
/// is always `None`; `table` is parsed from the message when present.
///
/// Other failures are classified from rusqlite's structured error data — the
/// [`rusqlite::Error`] variant and the SQLite primary/extended result code —
/// never from message text:
///
/// | SQLite condition | Structured evidence | `EFError` |
/// |---|---|---|
/// | unique / primary-key constraint | extended code `2067` / `1555` | `UniqueViolation` |
/// | database busy/locked, I/O error, cannot open, interrupted, disk full, out of memory | `ErrorCode::DatabaseBusy`/`DatabaseLocked`/`SystemIoFailure`/`CannotOpen`/`OperationInterrupted`/`DiskFull`/`OutOfMemory` | `Connection` |
/// | datatype mismatch, value/string too large | `ErrorCode::TypeMismatch`, `TooBig` | `TypeConversion` |
/// | bind parameter out of range | `ErrorCode::ParameterOutOfRange` | `Query` |
/// | statement aborted | `ErrorCode::OperationAborted` | `Transaction` |
/// | permission denied, read-only database, file is not a database | `ErrorCode::PermissionDenied`/`ReadOnly`/`NotADatabase` | `Configuration` |
/// | internal malfunction, library misuse, `NOTFOUND` file-control, no large-file support, statement authorization denied | `ErrorCode::InternalMalfunction`/`ApiMisuse`/`NotFound`/`NoLargeFileSupport`/`AuthorizationForStatementDenied` | `Provider` |
/// | `SQLITE_ERROR` (`code = 1`) | undifferentiated SQL error (see below) | `Query` |
/// | non-unique constraint, corrupt image, schema change, locking protocol | remaining codes | `Database` (structured diagnostics) |
/// | value decode/encode failure | `FromSqlConversionFailure`, `IntegralValueOutOfRange`, `Utf8Error`, `InvalidColumnType`, `NulError`, `ToSqlConversionFailure` | `TypeConversion` |
/// | projection/statement shape mismatch | `InvalidParameterName`/`InvalidColumnIndex`/`InvalidColumnName`/`InvalidDatabaseIndex`/`InvalidQuery`/`ExecuteReturnedResults`/`InvalidParameterCount`/`MultipleStatement`/`StatementChangedRows`/`QueryReturnedMoreThanOneRow`/`SqlInputError` | `Query` |
/// | no rows for a one-row query | `QueryReturnedNoRows` | `NotFound` |
/// | filesystem/handle configuration | `InvalidPath`, `SqliteSingleThreadedMode` | `Configuration` |
/// | driver panic or unknown future variant | `UnwindingPanic`, `_` | `Provider` |
///
/// `SQLITE_ERROR` (primary code `1`) is SQLite's undifferentiated bucket for
/// "SQL error or missing database" — syntax errors, missing tables/columns and
/// runtime aborts all share it — so the provider keeps the historical `Query`
/// category instead of guessing between a statement-shape defect (client
/// visible) and a schema defect (server side). The driver code and message are
/// preserved in the message for the operator log.
pub(crate) fn map_sqlite_error(e: rusqlite::Error) -> EFError {
    map_sqlite_error_with_sql(e, None)
}

/// Same as [`map_sqlite_error`], additionally attaching the failing SQL.
pub(crate) fn map_sqlite_error_with_sql(e: rusqlite::Error, sql: Option<&str>) -> EFError {
    classify_sqlite_error(&e, sql).with_source(Box::new(e))
}

fn classify_sqlite_error(e: &rusqlite::Error, sql: Option<&str>) -> EFError {
    if let rusqlite::Error::SqliteFailure(failure, message) = e {
        let extended = failure.extended_code;
        // Uniqueness stays the typed business conclusion (EF-2).
        if extended == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
            || extended == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
        {
            return EFError::UniqueViolation {
                constraint: None,
                table: message.as_deref().and_then(unique_violation_table),
            };
        }

        let detail = match message {
            Some(message) => format!("SQLite error {extended}: {message}"),
            None => format!("SQLite error {extended}: {failure}"),
        };
        return match failure.code {
            // Transient availability / transport failures: retrying can succeed
            // on a live connection and a new one may be needed.
            ErrorCode::DatabaseBusy
            | ErrorCode::DatabaseLocked
            | ErrorCode::SystemIoFailure
            | ErrorCode::CannotOpen
            | ErrorCode::OperationInterrupted
            | ErrorCode::DiskFull
            | ErrorCode::OutOfMemory => EFError::connection(detail),

            // Value/encoding level failures.
            ErrorCode::TypeMismatch | ErrorCode::TooBig => EFError::type_conversion(detail),

            // The statement bound a different parameter count/index than the
            // caller provided (projection/statement shape).
            ErrorCode::ParameterOutOfRange => EFError::query(detail),

            ErrorCode::OperationAborted => EFError::transaction(detail),

            // Environment/handle configuration: retrying cannot fix it.
            ErrorCode::PermissionDenied | ErrorCode::ReadOnly | ErrorCode::NotADatabase => {
                EFError::configuration(detail)
            }

            // Driver defects / unsupported capabilities, never client data.
            ErrorCode::InternalMalfunction
            | ErrorCode::ApiMisuse
            | ErrorCode::NotFound
            | ErrorCode::NoLargeFileSupport
            | ErrorCode::AuthorizationForStatementDenied => EFError::provider(detail),

            // The undifferentiated `SQLITE_ERROR` bucket: syntax errors,
            // missing tables/columns and runtime aborts cannot be told apart
            // without sniffing the message, so the historical `Query` category
            // is kept instead of guessing a narrower one.
            ErrorCode::Unknown => EFError::query(detail),

            // Non-unique constraint violations open a structured `Database`
            // (callers decide their meaning); the remaining codes are hard
            // database/driver conditions and keep the structured diagnostics.
            _ => EFError::database(database_info(message.as_deref(), failure, sql)),
        };
    }

    let detail = e.to_string();
    match e {
        rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::Utf8Error(..)
        | rusqlite::Error::NulError(..)
        | rusqlite::Error::InvalidColumnType(..)
        | rusqlite::Error::ToSqlConversionFailure(..) => EFError::type_conversion(detail),

        rusqlite::Error::InvalidParameterName(..)
        | rusqlite::Error::InvalidColumnIndex(..)
        | rusqlite::Error::InvalidColumnName(..)
        | rusqlite::Error::InvalidDatabaseIndex(..)
        | rusqlite::Error::ExecuteReturnedResults
        | rusqlite::Error::QueryReturnedMoreThanOneRow
        | rusqlite::Error::StatementChangedRows(..)
        | rusqlite::Error::InvalidQuery
        | rusqlite::Error::MultipleStatement
        | rusqlite::Error::InvalidParameterCount(..)
        | rusqlite::Error::SqlInputError { .. } => EFError::query(detail),

        rusqlite::Error::QueryReturnedNoRows => EFError::not_found(detail),

        rusqlite::Error::InvalidPath(..) | rusqlite::Error::SqliteSingleThreadedMode => {
            EFError::configuration(detail)
        }

        // `UnwindingPanic`, feature-gated module/UDF errors and future
        // variants: provider-level failures, never client data.
        _ => EFError::provider(detail),
    }
}

/// Builds the structured diagnostics for a SQLite driver failure.
fn database_info(
    message: Option<&str>,
    failure: &rusqlite::ffi::Error,
    sql: Option<&str>,
) -> DatabaseErrorInfo {
    let message = message
        .map(str::to_string)
        .unwrap_or_else(|| failure.to_string());
    let info = DatabaseErrorInfo::new(message).with_code(failure.extended_code.to_string());
    match sql {
        Some(sql) => info.with_sql(sql),
        None => info,
    }
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
        .map_err(|e| map_sqlite_error_with_sql(e, Some(sql)))?;
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
        .map_err(|e| map_sqlite_error_with_sql(e, Some(sql)))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| map_sqlite_error_with_sql(e, Some(sql)))?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{map_sqlite_error, map_sqlite_error_with_sql, unique_violation_table};
    use rusqlite::ffi;
    use rust_ef::error::EFError;

    fn failure(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(ffi::Error::new(code), Some("driver message".to_string()))
    }

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

    #[test]
    fn unique_and_primary_key_violations_stay_typed() {
        for code in [
            ffi::SQLITE_CONSTRAINT_UNIQUE,
            ffi::SQLITE_CONSTRAINT_PRIMARYKEY,
        ] {
            let error = rusqlite::Error::SqliteFailure(
                ffi::Error::new(code),
                Some("UNIQUE constraint failed: order_record.request_key".to_string()),
            );
            match map_sqlite_error(error) {
                EFError::UniqueViolation { table, constraint } => {
                    assert_eq!(table.as_deref(), Some("order_record"));
                    assert_eq!(constraint, None);
                }
                other => panic!("expected UniqueViolation, got {other:?}"),
            }
        }
    }

    #[test]
    fn transient_availability_failures_are_connections() {
        for code in [
            ffi::SQLITE_BUSY,
            ffi::SQLITE_LOCKED,
            ffi::SQLITE_IOERR,
            ffi::SQLITE_CANTOPEN,
            ffi::SQLITE_INTERRUPT,
            ffi::SQLITE_FULL,
            ffi::SQLITE_NOMEM,
        ] {
            let mapped = map_sqlite_error(failure(code));
            assert!(
                matches!(mapped, EFError::Connection(..)),
                "SQLite code {code} must be Connection, got {mapped:?}"
            );
        }
    }

    #[test]
    fn encoding_and_type_failures_are_type_conversions() {
        let mismatched = map_sqlite_error(failure(ffi::SQLITE_MISMATCH));
        assert!(matches!(mismatched, EFError::TypeConversion(..)));

        let decoded = map_sqlite_error(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad value",
            )),
        ));
        assert!(matches!(decoded, EFError::TypeConversion(..)));

        let out_of_range = map_sqlite_error(rusqlite::Error::IntegralValueOutOfRange(0, 1000));
        assert!(matches!(out_of_range, EFError::TypeConversion(..)));
    }

    #[test]
    fn undifferentiated_sqlite_error_keeps_the_query_category() {
        // `SQLITE_ERROR` is SQLite's bucket for syntax errors, missing tables
        // and runtime aborts; the provider keeps `Query` instead of guessing a
        // narrower category, and preserves the driver code in the message.
        let mapped = map_sqlite_error_with_sql(failure(ffi::SQLITE_ERROR), Some("SELECT 1"));
        match mapped {
            EFError::Query(message, _) => {
                assert!(message.contains("SQLite error 1"), "{message}");
                assert!(message.contains("driver message"), "{message}");
            }
            other => panic!("expected EFError::Query, got {other:?}"),
        }
    }

    #[test]
    fn non_unique_constraint_violations_keep_structured_diagnostics() {
        let mapped = map_sqlite_error(rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_CONSTRAINT),
            Some("NOT NULL constraint failed: order_record.tenant_id".to_string()),
        ));
        match mapped {
            EFError::Database(info, _) => {
                assert!(info.message.contains("NOT NULL"), "{info:?}");
                assert!(info.code.is_some(), "{info:?}");
            }
            other => panic!("expected EFError::Database, got {other:?}"),
        }
    }

    #[test]
    fn projection_shape_failures_are_queries() {
        for error in [
            rusqlite::Error::InvalidColumnIndex(3),
            rusqlite::Error::InvalidColumnName("missing".to_string()),
            rusqlite::Error::InvalidQuery,
            rusqlite::Error::ExecuteReturnedResults,
            rusqlite::Error::InvalidParameterCount(1, 2),
            rusqlite::Error::MultipleStatement,
            rusqlite::Error::SqlInputError {
                error: ffi::Error::new(ffi::SQLITE_ERROR),
                msg: "near \"SELEC\": syntax error".to_string(),
                sql: "SELEC 1".to_string(),
                offset: 0,
            },
        ] {
            let mapped = map_sqlite_error(error);
            assert!(
                matches!(mapped, EFError::Query(..)),
                "expected Query, got {mapped:?}"
            );
        }
    }

    #[test]
    fn missing_row_and_configuration_errors_stay_typed() {
        assert!(matches!(
            map_sqlite_error(rusqlite::Error::QueryReturnedNoRows),
            EFError::NotFound(..)
        ));
        assert!(matches!(
            map_sqlite_error(rusqlite::Error::SqliteSingleThreadedMode),
            EFError::Configuration(..)
        ));
        assert!(matches!(
            map_sqlite_error(rusqlite::Error::InvalidPath(std::path::PathBuf::from(""))),
            EFError::Configuration(..)
        ));
    }

    #[test]
    fn driver_defects_are_provider_level_not_client_data() {
        for error in [
            failure(ffi::SQLITE_INTERNAL),
            failure(ffi::SQLITE_MISUSE),
            rusqlite::Error::UnwindingPanic,
        ] {
            let mapped = map_sqlite_error(error);
            assert!(
                matches!(mapped, EFError::Provider(..)),
                "expected Provider, got {mapped:?}"
            );
        }
    }
}
