//! `rust-webx` integration for `rust-ef` (XR-1).
//!
//! Enabled by the **`webx` feature** (off by default, so the ORM keeps zero
//! dependency on a web framework unless asked). It provides
//! `impl From<EFError> for webx_core::error::Error`, which is what lets `?`
//! propagate a persistence failure out of a `rust-webx` handler
//! (`ActionResult<T>`) without `.map_err(...)`:
//!
//! ```
//! use rust_ef::error::EFError;
//! use webx_core::error::ActionResult;
//!
//! fn load() -> Result<u32, EFError> {
//!     Err(EFError::connection("pool exhausted"))
//! }
//!
//! fn handler() -> ActionResult<u32> {
//!     let value = load()?; // EFError -> webx::Error through the From impl
//!     Ok(value)
//! }
//! ```
//!
//! The impl has to live **here** (or in `rust-webx-core`): a third crate cannot
//! declare `impl From<EFError> for webx::Error` because of Rust's orphan rule
//! (`E0117` — both types are foreign to it). This module is therefore the one
//! implementation source: applications enable the `webx` feature and use `?`,
//! or call [`map_ef_error`] explicitly. If a shared cross-framework crate is
//! ever needed, it belongs in a dedicated `rust-core` under the Rust-Framework
//! organisation — not in a per-framework bridge crate.
//!
//! # Classification
//!
//! | `EFError` | HTTP | `webx::Error` variant | client-visible detail |
//! |---|---|---|---|
//! | `NotFound` | 404 | `NotFound` | the caller's business message |
//! | `ModelValidation` | 400 | `Validation` | the caller's business message |
//! | `ConcurrencyConflict` | 409 | `Conflict` | the caller's business message |
//! | `Query`, `TypeConversion` | 400 | `Validation` | fixed text |
//! | `UniqueViolation` | 409 | `Conflict` | fixed text |
//! | `Connection`, `Transaction` | 503 | `Status(503, _)` | fixed text |
//! | `Database` | 500 | `Internal` | none (`public_detail` is `None`) |
//! | `Migration`, `Configuration`, `Provider`, `ChangeTracking`, `Other` | 500 | `Internal` | none (`public_detail` is `None`) |
//!
//! # Sanitization contract
//!
//! Database-derived strings must never reach the client: driver messages carry
//! table/constraint names, SQL text and — for connection failures — connection
//! strings with credentials.
//!
//! * `UniqueViolation`: the driver metadata (`constraint`, `table`) is dropped.
//! * `Query`, `TypeConversion`, `Connection`, `Transaction`: the payload is
//!   dropped and replaced with fixed text.
//! * 5xx-class variants: the original message (and its `source` chain) is kept
//!   in `Error::Internal` for the operator log; `Error::public_detail` is `None`
//!   so the host serves a stable detail instead of echoing it.
//! * `NotFound`, `ModelValidation`, `ConcurrencyConflict`: the message is echoed
//!   because it is a business conclusion created by the caller (for example
//!   `EFError::concurrency_conflict("link state changed concurrently")`);
//!   keeping those messages client-safe is the caller's responsibility.

use crate::error::EFError;
use webx_core::error::Error;

/// Client-visible detail for a `400` caused by database-derived input problems.
///
/// The original `Query` / `TypeConversion` message is not echoed: it can carry
/// SQL text and column names.
const DETAIL_INVALID_REQUEST: &str =
    "the request could not be processed; the submitted data is invalid";

/// Client-visible detail for a unique-constraint race (`409`).
///
/// Deliberately generic: [`EFError::UniqueViolation`] carries driver metadata
/// (`constraint` / `table`) that must not leak to clients.
const DETAIL_UNIQUE_VIOLATION: &str = "a unique constraint was violated by a concurrent request";

/// Client-visible detail for a transient database outage (`503`).
const DETAIL_UNAVAILABLE: &str = "the database is temporarily unavailable; retry the request later";

/// Map a rust-ef error to a `rust-webx` error with RFC 7807 semantics.
///
/// This is the mapping behind `impl From<EFError> for webx::Error`; it is public
/// for call sites that need the mapping without a `From` conversion (for
/// example inside combinators). Enable the `webx` feature to get the conversion
/// as well.
///
/// ```
/// use rust_ef::error::EFError;
/// use rust_ef::webx::map_ef_error;
///
/// let error = map_ef_error(EFError::not_found("quota not found"));
/// assert_eq!(error.status_code(), 404);
/// ```
pub fn map_ef_error(error: EFError) -> Error {
    match &error {
        // Business conclusions authored by the caller: 404 / 400 / 409 with the
        // caller's own (client-safe) message.
        EFError::NotFound(message, _) => Error::NotFound(message.clone()),
        EFError::ModelValidation(message, _) => Error::Validation(message.clone()),
        EFError::ConcurrencyConflict(message, _) => Error::Conflict(message.clone()),

        // Database-derived payloads that must not be echoed: SQL text, column
        // names, driver messages. A fixed detail is served instead.
        EFError::Query(..) | EFError::TypeConversion(..) => {
            Error::Validation(DETAIL_INVALID_REQUEST.to_string())
        }

        // A unique-index race: driver metadata (constraint/table) is dropped.
        EFError::UniqueViolation { .. } => Error::Conflict(DETAIL_UNIQUE_VIOLATION.to_string()),

        // A driver error with structured diagnostics: the message can name
        // internal tables/columns/SQL, so only a fixed 500 goes to the client;
        // the full detail reaches the operator log.
        EFError::Database(info, source) => Error::Internal(operator_detail(
            "database",
            &info.to_string(),
            source.as_ref(),
        )),

        // Transient infrastructure failures: the client should retry. A fixed
        // detail replaces the driver text (connection strings may include
        // credentials).
        EFError::Connection(..) | EFError::Transaction(..) => {
            Error::Status(503, DETAIL_UNAVAILABLE.to_string())
        }

        // Server-side misconfiguration / internal defects: the full message and
        // its source chain go to the operator log through `Error::Internal`;
        // `public_detail` is `None` so nothing reaches the response body.
        EFError::Migration(message, source) => {
            Error::Internal(operator_detail("migration", message, source.as_ref()))
        }
        EFError::Configuration(message, source) => {
            Error::Internal(operator_detail("configuration", message, source.as_ref()))
        }
        EFError::Provider(message, source) => {
            Error::Internal(operator_detail("provider", message, source.as_ref()))
        }
        EFError::ChangeTracking(message, source) => {
            Error::Internal(operator_detail("change tracking", message, source.as_ref()))
        }
        EFError::Other(message, source) => {
            Error::Internal(operator_detail("other", message, source.as_ref()))
        }
    }
}

/// Operator-facing detail for a `500`: variant label, original message and the
/// full `source` chain.
///
/// This text is **never** serialized to clients: it is stored in
/// `Error::Internal`, whose `public_detail()` is `None`, and the host logs it
/// while serving a stable problem detail.
fn operator_detail(kind: &str, message: &str, source: Option<&crate::error::BoxError>) -> String {
    let mut detail = format!("rust-ef {kind} error: {message}");
    let mut cause: Option<&(dyn std::error::Error + 'static)> = match source {
        Some(error) => Some(error.as_ref()),
        None => None,
    };
    while let Some(error) = cause {
        detail.push_str(": ");
        detail.push_str(&error.to_string());
        cause = error.source();
    }
    detail
}

impl From<EFError> for Error {
    /// Convert a rust-ef failure into the `rust-webx` error type so `?` works in
    /// handlers that return [`ActionResult<T>`](webx_core::error::ActionResult).
    fn from(error: EFError) -> Self {
        map_ef_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webx_core::error::ActionResult;

    /// Strings that must never appear in a client-visible detail.
    const SECRET_CONSTRAINT: &str = "ux_orders_tenant_id";
    const SECRET_TABLE: &str = "orders_internal";
    const SECRET_SQL: &str = "SELECT * FROM orders_internal WHERE tenant_id = 42";
    const SECRET_CONNECTION: &str = "postgres://app:s3cr3t@db.internal:5432/travel";
    const SECRET_PATH: &str = "/srv/app/Migrations/20260101_init.sql";

    fn assert_client_safe(error: &Error, secrets: &[&str]) {
        let detail = error.public_detail().unwrap_or("");
        let display = error.to_string();
        for secret in secrets {
            assert!(
                !detail.contains(secret),
                "client detail leaked {secret:?}: {detail}"
            );
            assert!(
                !display.contains(secret),
                "client-visible Display leaked {secret:?}: {display}"
            );
        }
    }

    /// Every `EFError` variant, with its expected status. The table is the
    /// contract: a new rust-ef variant makes the mapping match fail to compile
    /// first, and this test list must be extended with it.
    #[test]
    fn every_variant_maps_to_its_expected_status() {
        let cases: Vec<(&str, EFError, u16)> = vec![
            ("NotFound", EFError::not_found("quota not found"), 404),
            (
                "ModelValidation",
                EFError::model_validation("amount must be positive"),
                400,
            ),
            (
                "ConcurrencyConflict",
                EFError::concurrency_conflict("link state changed concurrently"),
                409,
            ),
            ("Query", EFError::query(SECRET_SQL), 400),
            (
                "TypeConversion",
                EFError::type_conversion("SUM result is not f64"),
                400,
            ),
            (
                "UniqueViolation",
                EFError::unique_violation(
                    Some(SECRET_CONSTRAINT.into()),
                    Some(SECRET_TABLE.into()),
                ),
                409,
            ),
            ("Connection", EFError::connection(SECRET_CONNECTION), 503),
            (
                "Transaction",
                EFError::transaction("transaction connection is already consumed"),
                503,
            ),
            ("Migration", EFError::migration(SECRET_PATH), 500),
            (
                "Configuration",
                EFError::configuration("database provider is not configured"),
                500,
            ),
            (
                "Provider",
                EFError::provider("unsupported column type"),
                500,
            ),
            (
                "ChangeTracking",
                EFError::change_tracking("snapshot is missing"),
                500,
            ),
            ("Other", EFError::other("unexpected driver failure"), 500),
            (
                "Database",
                EFError::database(
                    crate::error::DatabaseErrorInfo::new("db error")
                        .with_sqlstate("23502")
                        .with_table(SECRET_TABLE)
                        .with_sql(SECRET_SQL),
                ),
                500,
            ),
        ];

        // 14 variants in rust-ef 2.5 (`crates/core/src/error.rs`). The count is
        // asserted so a variant added upstream without updating this list fails
        // loudly instead of being silently untested.
        assert_eq!(cases.len(), 14, "rust-ef 2.5 EFError has 14 variants");

        for (name, error, expected_status) in cases {
            let mapped = map_ef_error(error);
            assert_eq!(mapped.status_code(), expected_status, "status for {name}");
        }
    }

    #[test]
    fn unique_violation_is_409_and_hides_driver_metadata() {
        let error = map_ef_error(EFError::unique_violation(
            Some(SECRET_CONSTRAINT.into()),
            Some(SECRET_TABLE.into()),
        ));
        assert_eq!(error.status_code(), 409);
        assert!(matches!(error, Error::Conflict(_)));
        assert_client_safe(&error, &[SECRET_CONSTRAINT, SECRET_TABLE]);
        assert!(error.public_detail().is_some());
    }

    #[test]
    fn connection_and_transaction_are_503_with_fixed_detail() {
        for error in [
            EFError::connection(SECRET_CONNECTION),
            EFError::transaction("transaction connection is already consumed"),
        ] {
            let mapped = map_ef_error(error);
            assert_eq!(mapped.status_code(), 503);
            assert!(matches!(mapped, Error::Status(503, _)));
            assert_client_safe(&mapped, &[SECRET_CONNECTION, "s3cr3t"]);
            assert!(mapped.public_detail().is_some());
        }
    }

    #[test]
    fn query_and_type_conversion_are_400_without_sql() {
        for error in [
            EFError::query(SECRET_SQL),
            EFError::type_conversion(SECRET_SQL),
        ] {
            let mapped = map_ef_error(error);
            assert_eq!(mapped.status_code(), 400);
            assert!(matches!(mapped, Error::Validation(_)));
            assert_client_safe(&mapped, &[SECRET_SQL, "orders_internal", "SELECT"]);
        }
    }

    #[test]
    fn business_authored_messages_are_preserved() {
        let mapped = map_ef_error(EFError::concurrency_conflict(
            "link state changed concurrently",
        ));
        assert_eq!(
            mapped.public_detail(),
            Some("link state changed concurrently")
        );

        let mapped = map_ef_error(EFError::not_found("quota not found"));
        assert_eq!(mapped.status_code(), 404);
        assert_eq!(mapped.public_detail(), Some("quota not found"));

        let mapped = map_ef_error(EFError::model_validation("amount must be positive"));
        assert_eq!(mapped.status_code(), 400);
        assert_eq!(mapped.public_detail(), Some("amount must be positive"));
    }

    #[test]
    fn server_side_variants_are_500_and_keep_detail_out_of_the_response() {
        let cases = [
            EFError::migration(SECRET_PATH),
            EFError::configuration("database provider is not configured"),
            EFError::provider("unsupported column type"),
            EFError::change_tracking("snapshot is missing"),
            EFError::other("unexpected driver failure"),
            EFError::database(
                crate::error::DatabaseErrorInfo::new("db error")
                    .with_sqlstate("23502")
                    .with_sql(SECRET_SQL),
            ),
        ];
        for error in cases {
            let mapped = map_ef_error(error);
            assert_eq!(mapped.status_code(), 500);
            assert!(matches!(mapped, Error::Internal(_)));
            // `public_detail` is `None`: the host serves a stable detail and logs
            // the internal text instead.
            assert_eq!(mapped.public_detail(), None);
        }
    }

    #[test]
    fn migration_detail_preserves_message_for_the_operator_log() {
        let mapped = map_ef_error(EFError::migration(SECRET_PATH));
        // Operator-only text (never in the response body).
        assert!(mapped.to_string().contains(SECRET_PATH));
        assert!(mapped.to_string().contains("rust-ef migration error:"));
    }

    #[test]
    fn mapped_error_keeps_the_source_chain_for_the_operator_log() {
        let error = EFError::other("save failed").with_source("driver: disk I/O error".into());
        let mapped = map_ef_error(error);
        let display = mapped.to_string();
        assert!(display.contains("save failed"), "got: {display}");
        assert!(display.contains("driver: disk I/O error"), "got: {display}");
        assert_eq!(mapped.public_detail(), None);
    }

    /// Acceptance for XR-1: `?` converts `EFError` directly inside a function
    /// that returns the `rust-webx` action result.
    #[test]
    fn question_mark_propagates_ef_error_into_action_result() {
        fn persist() -> Result<u32, EFError> {
            Err(EFError::unique_violation(
                Some(SECRET_CONSTRAINT.into()),
                Some(SECRET_TABLE.into()),
            ))
        }

        fn handler_with_question_mark() -> ActionResult<u32> {
            let value = persist()?; // EFError -> webx::Error via From
            Ok(value)
        }

        let error = handler_with_question_mark().expect_err("handler must fail");
        assert_eq!(error.status_code(), 409);
        assert_client_safe(&error, &[SECRET_CONSTRAINT, SECRET_TABLE]);

        // `Ok` values pass through untouched.
        let ok: ActionResult<u32> = Ok(7u32);
        assert!(matches!(ok, Ok(7)));
    }

    #[test]
    fn from_and_map_ef_error_agree() {
        let from: Error = EFError::not_found("quota not found").into();
        let explicit = map_ef_error(EFError::not_found("quota not found"));
        assert_eq!(from.status_code(), explicit.status_code());
        assert_eq!(from.public_detail(), explicit.public_detail());
    }
}
