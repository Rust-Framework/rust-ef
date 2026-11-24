//! PostgreSQL error mapping (EF-ERR-01).
//!
//! A bare `tokio_postgres::Error` prints as "db error" because the driver keeps
//! SQLSTATE, detail, hint, table/column/constraint and the message in structured
//! fields. Mapping copies them into [`EFError::Database`] so a production log
//! identifies the failed statement without a second connection.
//!
//! # Classification
//!
//! `tokio_postgres` 0.7 exposes no public accessor for its internal error
//! `Kind`; the discriminating facts are the server payload
//! ([`tokio_postgres::Error::as_db_error`]), the public
//! [`tokio_postgres::Error::is_closed`] flag and the `std::error::Error`
//! source chain. The provider therefore maps:
//!
//! | Driver condition | Structured evidence | `EFError` |
//! |---|---|---|
//! | Server error (unique/primary violation) | SQLSTATE `23505` | `UniqueViolation` |
//! | Server error (any other SQLSTATE) | `as_db_error()` payload | `Database` |
//! | Connection closed | `is_closed()` | `Connection` |
//! | Transport / socket / connect / response-parse failure | source chain contains `std::io::Error` | `Connection` |
//! | Value encoding/decoding failure | source chain contains `postgres_types::WrongType` or `WasNull` | `TypeConversion` |
//! | Remaining driver-local failures | no structured signal (projection/column arity, unexpected protocol message, custom `ToSql`/`FromSql` payloads) | `Query` (documented residual) |
//!
//! Connection-loss errors never collapse to `Query` (which a web adapter maps to
//! `400`): the closed flag and the `io::Error` source chain are both structural.
//! Connect timeouts arrive as the driver's connect kind wrapping an
//! `io::Error`, and pool-acquisition timeouts are mapped to `Connection` by
//! `provider.rs`; neither needs message text. The residual
//! `Query` bucket is deliberate — `tokio-postgres` splits projection-shape
//! failures (`Kind::Column`/`ColumnCount`/`Parameters`/`RowCount`) from
//! protocol/opaque failures only through its private `Kind`, so guessing would
//! be worse than keeping the historical category. Connect-time
//! authentication/TLS/config failures do not reach this function: the provider
//! maps them to `Connection`/`Configuration` while creating the pool.

use rust_ef::error::{DatabaseErrorInfo, EFError};
use std::error::Error as StdError;
use tokio_postgres::error::SqlState;

/// Mapping category for a driver-local `tokio_postgres::Error` (no server
/// payload). Kept separate from [`EFError`] construction so the classification
/// is unit-testable without constructing the driver's opaque error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PgFailureKind {
    /// Transport / connection failure (closed connection, I/O error, socket or
    /// protocol write failure).
    Connection,
    /// Encoding or decoding of a Rust value failed.
    TypeConversion,
    /// Client-side statement/projection shape failure, or an opaque driver
    /// failure that exposes no structured signal (see the module docs).
    Query,
}

/// Maps a `tokio_postgres` error to a typed [`EFError`].
///
/// SQLSTATE `23505` (unique violation) is promoted to
/// [`EFError::UniqueViolation`], using PostgreSQL's structured
/// `table`/`constraint` fields instead of parsing the message. Every other
/// server error keeps its structured diagnostics in [`EFError::Database`];
/// driver-local failures are classified by [`classify_driver_failure`].
pub(crate) fn map_pg_error(e: tokio_postgres::Error, context: &str, sql: &str) -> EFError {
    if e.code() == Some(&SqlState::UNIQUE_VIOLATION) {
        let db_error = e.as_db_error();
        return EFError::UniqueViolation {
            constraint: db_error.and_then(|d| d.constraint()).map(str::to_string),
            table: db_error.and_then(|d| d.table()).map(str::to_string),
        };
    }
    let info = e.as_db_error().map(|db| {
        let mut info = DatabaseErrorInfo::new(db.message()).with_sqlstate(db.code().code());
        if let Some(detail) = db.detail() {
            info = info.with_detail(detail);
        }
        if let Some(hint) = db.hint() {
            info = info.with_hint(hint);
        }
        if let Some(table) = db.table() {
            info = info.with_table(table);
        }
        if let Some(column) = db.column() {
            info = info.with_column(column);
        }
        if let Some(constraint) = db.constraint() {
            info = info.with_constraint(constraint);
        }
        info.with_sql(sql)
    });
    match info {
        Some(info) => EFError::database(info).with_source(Box::new(e)),
        None => {
            let detail = format!("{context}: {e}");
            let mapped = match classify_driver_failure(e.is_closed(), e.source()) {
                PgFailureKind::Connection => EFError::connection(detail),
                PgFailureKind::TypeConversion => EFError::type_conversion(detail),
                PgFailureKind::Query => EFError::query(detail),
            };
            mapped.with_source(Box::new(e))
        }
    }
}

/// Classifies a driver-local error (no server payload) from structured facts.
fn classify_driver_failure(
    closed: bool,
    source: Option<&(dyn StdError + 'static)>,
) -> PgFailureKind {
    if closed || source_chain_has::<std::io::Error>(source) {
        PgFailureKind::Connection
    } else if source_chain_has::<tokio_postgres::types::WrongType>(source)
        || source_chain_has::<tokio_postgres::types::WasNull>(source)
    {
        PgFailureKind::TypeConversion
    } else {
        PgFailureKind::Query
    }
}

/// Returns whether `source` or any error in its cause chain is a `T`.
fn source_chain_has<T: StdError + 'static>(source: Option<&(dyn StdError + 'static)>) -> bool {
    let mut current = source;
    while let Some(error) = current {
        if error.is::<T>() {
            return true;
        }
        current = error.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wraps an inner error so the chain walk is exercised beyond one hop.
    #[derive(Debug)]
    struct Wrapper(std::io::Error);

    impl std::fmt::Display for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("wrapped")
        }
    }

    impl StdError for Wrapper {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn sqlstate_is_promoted_for_unique_violations() {
        // Constructing a `tokio_postgres::Error` with a DB payload is not
        // possible from outside the crate; the structured fields are covered by
        // the ignored live-PostgreSQL parity test. This unit test pins the
        // mapping contract for the code/`Display` shape.
        let info = DatabaseErrorInfo::new("duplicate key value violates unique constraint \"ux\"")
            .with_sqlstate("23505")
            .with_constraint("ux")
            .with_table("orders")
            .with_sql("INSERT INTO orders DEFAULT VALUES");
        let rendered = info.to_string();
        assert!(rendered.contains("23505"), "{rendered}");
        assert!(rendered.contains("orders"), "{rendered}");
        assert!(rendered.contains("ux"), "{rendered}");
    }

    #[test]
    fn closed_connection_is_a_connection_failure() {
        assert_eq!(
            classify_driver_failure(true, None),
            PgFailureKind::Connection
        );
    }

    #[test]
    fn io_source_is_a_connection_failure() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset by peer");
        assert_eq!(
            classify_driver_failure(false, Some(&io)),
            PgFailureKind::Connection
        );
    }

    #[test]
    fn nested_io_source_is_a_connection_failure() {
        let wrapper = Wrapper(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "connection closed",
        ));
        assert_eq!(
            classify_driver_failure(false, Some(&wrapper)),
            PgFailureKind::Connection
        );
    }

    #[test]
    fn wrong_type_is_a_type_conversion_failure() {
        let wrong =
            tokio_postgres::types::WrongType::new::<String>(tokio_postgres::types::Type::INT4);
        assert_eq!(
            classify_driver_failure(false, Some(&wrong)),
            PgFailureKind::TypeConversion
        );
    }

    #[test]
    fn unexpected_null_is_a_type_conversion_failure() {
        assert_eq!(
            classify_driver_failure(false, Some(&tokio_postgres::types::WasNull)),
            PgFailureKind::TypeConversion
        );
    }

    #[test]
    fn opaque_driver_failure_keeps_the_query_category() {
        // `tokio_postgres` exposes projection-shape failures (`Column`,
        // `ColumnCount`, `Parameters`, `RowCount`) and opaque protocol failures
        // through the same public surface; the documented residual is `Query`.
        #[derive(Debug)]
        struct CustomError;

        impl std::fmt::Display for CustomError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("custom")
            }
        }

        impl StdError for CustomError {}

        assert_eq!(classify_driver_failure(false, None), PgFailureKind::Query);
        assert_eq!(
            classify_driver_failure(false, Some(&CustomError)),
            PgFailureKind::Query
        );
    }
}
