//! Error types for Rust Entity Framework (rust-ef).
//!
//! Each variant carries an optional `#[source]` for preserving the original
//! error chain, enabling programmatic inspection via `std::error::Error::source()`.
//! Convenience constructors (`EFError::query(msg)`, etc.) keep call sites concise;
//! `with_source` and `with_sql` builders attach context after construction.

use std::fmt;

use thiserror::Error;

/// Boxed error suitable for `EFError::source`.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Maximum number of SQL characters kept in [`DatabaseErrorInfo::sql`].
pub const MAX_SQL_PREVIEW_LEN: usize = 512;

/// Structured diagnostics for a database/driver error (EF-ERR-01).
///
/// Providers attach this to [`EFError::Database`] so a failed statement keeps
/// its SQLSTATE (or vendor code), driver message, affected table/column/
/// constraint and the SQL that failed. The information is operator-facing: a
/// web adapter must still map the error to a generic 5xx response, because the
/// message can name internal objects.
///
/// The struct is `Default` and every optional field is additive, so a provider
/// can fill only what its driver exposes (for example SQLite reports a code and
/// message but no constraint name).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseErrorInfo {
    /// SQLSTATE code (e.g. `23505` on PostgreSQL, `42S02` on MySQL).
    pub sqlstate: Option<String>,
    /// Vendor-specific error code (e.g. MySQL `1062`, SQLite `2067`).
    pub code: Option<String>,
    /// Driver message, verbatim.
    pub message: String,
    /// Optional driver detail (`DETAIL: ...` on PostgreSQL).
    pub detail: Option<String>,
    /// Optional driver hint (`HINT: ...` on PostgreSQL).
    pub hint: Option<String>,
    /// Constraint/index name when reported by the driver.
    pub constraint: Option<String>,
    /// Table name when reported by the driver.
    pub table: Option<String>,
    /// Column name when reported by the driver.
    pub column: Option<String>,
    /// The SQL statement that failed (bounded by the provider).
    pub sql: Option<String>,
}

impl DatabaseErrorInfo {
    /// Creates an info value from a driver message; use the builder methods for
    /// the remaining fields.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            ..Self::default()
        }
    }

    /// Sets the SQLSTATE code.
    pub fn with_sqlstate(mut self, sqlstate: impl Into<String>) -> Self {
        self.sqlstate = Some(sqlstate.into());
        self
    }

    /// Sets the vendor error code.
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Sets the driver detail text.
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Sets the driver hint text.
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Sets the constraint/index name.
    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraint = Some(constraint.into());
        self
    }

    /// Sets the table name.
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }

    /// Sets the column name.
    pub fn with_column(mut self, column: impl Into<String>) -> Self {
        self.column = Some(column.into());
        self
    }

    /// Sets the SQL statement that failed.
    ///
    /// The statement is bounded to [`MAX_SQL_PREVIEW_LEN`] characters so a
    /// multi-megabyte migration script does not end up in every log line.
    pub fn with_sql(mut self, sql: impl Into<String>) -> Self {
        let sql = sql.into();
        if sql.trim().is_empty() {
            return self;
        }
        let bounded = if sql.chars().count() > MAX_SQL_PREVIEW_LEN {
            let mut preview: String = sql.chars().take(MAX_SQL_PREVIEW_LEN).collect();
            preview.push('…');
            preview
        } else {
            sql
        };
        self.sql = Some(bounded);
        self
    }

    /// Whether the error is a syntax/statement error rather than a transport
    /// failure (`SQLSTATE` class `42` / MySQL `1064` / SQLite `1`).
    pub fn is_syntax_error(&self) -> bool {
        self.sqlstate
            .as_deref()
            .is_some_and(|state| state.starts_with("42"))
            || self.code.as_deref() == Some("1064")
    }
}

impl fmt::Display for DatabaseErrorInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database error")?;
        let mut context: Vec<String> = Vec::new();
        if let Some(state) = &self.sqlstate {
            context.push(format!("SQLSTATE {state}"));
        }
        if let Some(code) = &self.code {
            context.push(format!("code {code}"));
        }
        if !context.is_empty() {
            write!(f, " [{}]", context.join(", "))?;
        }
        write!(f, ": {}", self.message)?;
        if let Some(table) = &self.table {
            write!(f, "; table={table}")?;
        }
        if let Some(column) = &self.column {
            write!(f, "; column={column}")?;
        }
        if let Some(constraint) = &self.constraint {
            write!(f, "; constraint={constraint}")?;
        }
        if let Some(detail) = &self.detail {
            write!(f, "; detail: {detail}")?;
        }
        if let Some(hint) = &self.hint {
            write!(f, "; hint: {hint}")?;
        }
        if let Some(sql) = &self.sql {
            write!(f, "; sql: {sql}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DatabaseErrorInfo {}

/// Represents all possible errors that can occur in rust-ef operations.
#[derive(Error, Debug)]
pub enum EFError {
    /// Database connection error.
    #[error("database connection error: {0}")]
    Connection(String, #[source] Option<BoxError>),

    /// Query execution error.
    #[error("query error: {0}")]
    Query(String, #[source] Option<BoxError>),

    /// A driver/database error with structured diagnostics (SQLSTATE, code,
    /// affected table/column/constraint and the failing SQL).
    ///
    /// Providers produce this instead of flattening the driver error to
    /// `Query("... db error")`, so logs and `Display` keep the information
    /// needed to diagnose a failed statement. Application code should keep
    /// matching on [`EFError::UniqueViolation`] for uniqueness races.
    ///
    /// The payload is boxed so `EFError` stays small: the diagnostics struct
    /// carries nine optional strings, and inlining it made every `Result` in
    /// the workspace trippingly large (`clippy::result_large_err`).
    #[error("{0}")]
    Database(Box<DatabaseErrorInfo>, #[source] Option<BoxError>),

    /// Entity not found error.
    #[error("entity not found: {0}")]
    NotFound(String, #[source] Option<BoxError>),

    /// Model validation error.
    #[error("model validation error: {0}")]
    ModelValidation(String, #[source] Option<BoxError>),

    /// Migration error.
    #[error("migration error: {0}")]
    Migration(String, #[source] Option<BoxError>),

    /// Provider-specific error.
    #[error("provider error: {0}")]
    Provider(String, #[source] Option<BoxError>),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Configuration(String, #[source] Option<BoxError>),

    /// Change tracking error.
    #[error("change tracking error: {0}")]
    ChangeTracking(String, #[source] Option<BoxError>),

    /// Transaction error.
    #[error("transaction error: {0}")]
    Transaction(String, #[source] Option<BoxError>),

    /// Concurrency conflict error.
    #[error("concurrency conflict: {0}")]
    ConcurrencyConflict(String, #[source] Option<BoxError>),

    /// A unique / primary-key constraint was violated.
    ///
    /// `constraint` is the constraint or index name when the driver exposes one
    /// (MySQL `ER_DUP_ENTRY`, PostgreSQL SQLSTATE `23505`). SQLite does not
    /// report constraint names, so it is `None` for the SQLite provider.
    /// `table` is the table named by the driver in its error message, when
    /// extractable.
    ///
    /// This is the typed replacement for sniffing driver messages
    /// (`"Duplicate entry"`, `"UNIQUE constraint failed"`, …) in application
    /// code.
    #[error("unique constraint violation (table: {table:?}, constraint: {constraint:?})")]
    UniqueViolation {
        /// Constraint / index name, when reported by the driver.
        constraint: Option<String>,
        /// Table named by the driver, when extractable.
        table: Option<String>,
    },

    /// Type conversion error.
    #[error("type conversion error: {0}")]
    TypeConversion(String, #[source] Option<BoxError>),

    /// General / unknown error.
    #[error("{0}")]
    Other(String, #[source] Option<BoxError>),
}

/// Stable, programmatic error code for `EFError` classification.
///
/// Unlike the `EFError` enum (which may gain variants over time), `EFErrorCode`
/// is a stable, `Copy` taxonomy that callers can match on for retry logic,
/// metrics, or user-facing routing. Sub-categories (e.g. `ConnectionTimeout`
/// vs `ConnectionRefused`) are derived from message patterns within each
/// `EFError` variant — see [`EFError::code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EFErrorCode {
    ConnectionRefused,
    ConnectionTimeout,
    QuerySyntax,
    QueryTimeout,
    EntityNotFound,
    ModelValidation,
    MigrationConflict,
    ProviderUnsupported,
    ConfigurationInvalid,
    ChangeTrackingCorrupted,
    TransactionAborted,
    TransactionDeadlock,
    ConcurrencyConflict,
    UniqueViolation,
    TypeConversionFailed,
    Unknown,
}

impl EFError {
    pub fn connection(msg: impl Into<String>) -> Self {
        EFError::Connection(msg.into(), None)
    }

    pub fn query(msg: impl Into<String>) -> Self {
        EFError::Query(msg.into(), None)
    }

    /// Creates a [`EFError::Database`] from structured driver diagnostics.
    pub fn database(info: DatabaseErrorInfo) -> Self {
        EFError::Database(Box::new(info), None)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        EFError::NotFound(msg.into(), None)
    }

    pub fn model_validation(msg: impl Into<String>) -> Self {
        EFError::ModelValidation(msg.into(), None)
    }

    pub fn migration(msg: impl Into<String>) -> Self {
        EFError::Migration(msg.into(), None)
    }

    pub fn provider(msg: impl Into<String>) -> Self {
        EFError::Provider(msg.into(), None)
    }

    pub fn configuration(msg: impl Into<String>) -> Self {
        EFError::Configuration(msg.into(), None)
    }

    pub fn change_tracking(msg: impl Into<String>) -> Self {
        EFError::ChangeTracking(msg.into(), None)
    }

    pub fn transaction(msg: impl Into<String>) -> Self {
        EFError::Transaction(msg.into(), None)
    }

    pub fn concurrency_conflict(msg: impl Into<String>) -> Self {
        EFError::ConcurrencyConflict(msg.into(), None)
    }

    /// Creates a [`EFError::UniqueViolation`] from optional driver metadata.
    pub fn unique_violation(constraint: Option<String>, table: Option<String>) -> Self {
        EFError::UniqueViolation { constraint, table }
    }

    pub fn type_conversion(msg: impl Into<String>) -> Self {
        EFError::TypeConversion(msg.into(), None)
    }

    pub fn other(msg: impl Into<String>) -> Self {
        EFError::Other(msg.into(), None)
    }

    /// Attaches a source error to this `EFError`, preserving the error chain.
    ///
    /// [`EFError::UniqueViolation`] has no source slot (its metadata comes from
    /// the driver itself), so the source is dropped when applied to it.
    pub fn with_source(self, source: BoxError) -> Self {
        match self {
            EFError::Connection(m, _) => EFError::Connection(m, Some(source)),
            EFError::Query(m, _) => EFError::Query(m, Some(source)),
            EFError::Database(info, _) => EFError::Database(info, Some(source)),

            EFError::NotFound(m, _) => EFError::NotFound(m, Some(source)),
            EFError::ModelValidation(m, _) => EFError::ModelValidation(m, Some(source)),
            EFError::Migration(m, _) => EFError::Migration(m, Some(source)),
            EFError::Provider(m, _) => EFError::Provider(m, Some(source)),
            EFError::Configuration(m, _) => EFError::Configuration(m, Some(source)),
            EFError::ChangeTracking(m, _) => EFError::ChangeTracking(m, Some(source)),
            EFError::Transaction(m, _) => EFError::Transaction(m, Some(source)),
            EFError::ConcurrencyConflict(m, _) => EFError::ConcurrencyConflict(m, Some(source)),
            EFError::UniqueViolation { constraint, table } => {
                EFError::UniqueViolation { constraint, table }
            }
            EFError::TypeConversion(m, _) => EFError::TypeConversion(m, Some(source)),
            EFError::Other(m, _) => EFError::Other(m, Some(source)),
        }
    }

    /// Extracts the human-readable message without the source chain.
    pub fn message(&self) -> &str {
        match self {
            EFError::Connection(m, _) => m,
            EFError::Query(m, _) => m,
            EFError::Database(info, _) => &info.message,
            EFError::NotFound(m, _) => m,
            EFError::ModelValidation(m, _) => m,
            EFError::Migration(m, _) => m,
            EFError::Provider(m, _) => m,
            EFError::Configuration(m, _) => m,
            EFError::ChangeTracking(m, _) => m,
            EFError::Transaction(m, _) => m,
            EFError::ConcurrencyConflict(m, _) => m,
            EFError::UniqueViolation { .. } => "unique constraint violation",
            EFError::TypeConversion(m, _) => m,
            EFError::Other(m, _) => m,
        }
    }

    /// Returns a stable [`EFErrorCode`] for programmatic dispatch.
    ///
    /// Sub-categories within a variant are detected via case-insensitive
    /// message patterns (e.g. `"timeout"` → `ConnectionTimeout`). Messages
    /// are stable internal strings produced by rust-ef's own call sites;
    /// user-supplied messages from [`EFError::other`] fall back to `Unknown`.
    pub fn code(&self) -> EFErrorCode {
        let msg = self.message().to_ascii_lowercase();
        match self {
            EFError::Connection(_, _) => {
                if msg.contains("timeout") {
                    EFErrorCode::ConnectionTimeout
                } else {
                    EFErrorCode::ConnectionRefused
                }
            }
            EFError::Query(_, _) => {
                if msg.contains("timeout") {
                    EFErrorCode::QueryTimeout
                } else {
                    EFErrorCode::QuerySyntax
                }
            }
            EFError::Database(info, _) => {
                let state = info.sqlstate.as_deref().unwrap_or_default();
                if state.starts_with("08") {
                    EFErrorCode::ConnectionRefused
                } else if state.starts_with("22") {
                    EFErrorCode::TypeConversionFailed
                } else if state.starts_with("40") {
                    EFErrorCode::TransactionDeadlock
                } else {
                    // SQLSTATE class 42 (syntax/access rule) and every other
                    // statement failure share the query-error category.
                    EFErrorCode::QuerySyntax
                }
            }
            EFError::NotFound(_, _) => EFErrorCode::EntityNotFound,
            EFError::ModelValidation(_, _) => EFErrorCode::ModelValidation,
            EFError::Migration(_, _) => EFErrorCode::MigrationConflict,
            EFError::Provider(_, _) => EFErrorCode::ProviderUnsupported,
            EFError::Configuration(_, _) => EFErrorCode::ConfigurationInvalid,
            EFError::ChangeTracking(_, _) => EFErrorCode::ChangeTrackingCorrupted,
            EFError::Transaction(_, _) => {
                if msg.contains("deadlock") {
                    EFErrorCode::TransactionDeadlock
                } else {
                    EFErrorCode::TransactionAborted
                }
            }
            EFError::ConcurrencyConflict(_, _) => EFErrorCode::ConcurrencyConflict,
            EFError::UniqueViolation { .. } => EFErrorCode::UniqueViolation,
            EFError::TypeConversion(_, _) => EFErrorCode::TypeConversionFailed,
            EFError::Other(_, _) => EFErrorCode::Unknown,
        }
    }
}

/// Result type alias for rust-ef operations.
pub type EFResult<T> = Result<T, EFError>;

/// Client-visible detail for a query/shape failure (`400`).
pub const DETAIL_INVALID_REQUEST: &str = "the submitted data is invalid";

/// Client-visible detail for a unique-violation race (`409`); constraint/table
/// names stay server-side.
pub const DETAIL_UNIQUE_VIOLATION: &str =
    "a unique constraint was violated by a concurrent request";

/// Client-visible detail for a transient database outage (`503`).
pub const DETAIL_UNAVAILABLE: &str = "the database is temporarily unavailable; retry later";

/// Whether a structured driver error describes a condition a caller can retry.
///
/// This is the **single source** of the transient classification used by the
/// shared error carrier (`AppError`) and by Web adapters; it moves the predicate
/// out of `rust-webx-ef` so a non-Web consumer (CLI, worker) reaches the same
/// conclusion.
///
/// * `SQLSTATE` class `08` — connection exception;
/// * `SQLSTATE` class `40` — serialization failure / deadlock;
/// * `SQLSTATE` `57P01`/`57P02`/`57P03` — shutdown, crash, cannot connect;
/// * MySQL vendor codes `1205` (lock wait timeout) / `1213` (deadlock);
/// * SQLite has no SQLSTATE: its extended result code embeds the primary code
///   in the low byte (`SQLITE_BUSY = 5`, `SQLITE_LOCKED = 6`), so `5`/`6`
///   (including `261`/`517`/`262`) qualify.
pub fn database_error_is_transient(info: &DatabaseErrorInfo) -> bool {
    if let Some(sqlstate) = info.sqlstate.as_deref() {
        if sqlstate.starts_with("08")
            || sqlstate.starts_with("40")
            || matches!(sqlstate, "57P01" | "57P02" | "57P03")
        {
            return true;
        }
    }
    match info.code.as_deref() {
        Some("1205" | "1213") => true,
        // SQLite is the supported provider that reports no SQLSTATE; its
        // extended code embeds the primary code in the low eight bits.
        Some(code) if info.sqlstate.is_none() => code
            .parse::<i32>()
            .map(|code| matches!(code & 0xFF, 5 | 6))
            .unwrap_or(false),
        _ => false,
    }
}

/// Operator-facing detail for a server error: the EF message plus its source
/// chain. Never use this as a client-visible detail.
fn operator_detail(error: &EFError) -> String {
    let mut detail = error.to_string();
    let mut cause = std::error::Error::source(error);
    while let Some(error) = cause {
        detail.push_str(": ");
        detail.push_str(&error.to_string());
        cause = error.source();
    }
    detail
}

/// Convert a persistence failure into the shared application error carrier.
///
/// This is the rust-ef ↔ rust-core boundary: callers that return
/// `rust_core::Result<T>` can use `?` directly, without a Web adapter. Business
/// conclusions keep their message; driver-derived payloads are replaced by
/// fixed text (SQL, column, constraint and connection details never reach the
/// client); transient conditions become a retryable `503`; everything else is a
/// hidden `500` whose operator detail carries the full EF message/source chain.
impl From<EFError> for rust_core::AppError {
    fn from(error: EFError) -> Self {
        match error {
            EFError::NotFound(message, _) => rust_core::AppError::not_found(message),
            EFError::ModelValidation(message, _) => rust_core::AppError::validation(message),
            EFError::ConcurrencyConflict(message, _) => rust_core::AppError::conflict(message),

            error @ (EFError::Query(..) | EFError::TypeConversion(..)) => {
                rust_core::AppError::validation(DETAIL_INVALID_REQUEST)
                    .with_operator(operator_detail(&error))
            }
            error @ EFError::UniqueViolation { .. } => {
                rust_core::AppError::conflict(DETAIL_UNIQUE_VIOLATION)
                    .with_operator(operator_detail(&error))
            }

            error @ (EFError::Connection(..) | EFError::Transaction(..)) => {
                rust_core::AppError::unavailable(DETAIL_UNAVAILABLE)
                    .with_operator(operator_detail(&error))
            }
            ref error @ EFError::Database(ref info, _) if database_error_is_transient(info) => {
                rust_core::AppError::unavailable(DETAIL_UNAVAILABLE)
                    .with_operator(operator_detail(error))
            }

            other => rust_core::AppError::internal(operator_detail(&other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_violation_exposes_driver_metadata() {
        let err = EFError::unique_violation(Some("ux_order".into()), Some("order_record".into()));
        assert_eq!(err.code(), EFErrorCode::UniqueViolation);
        assert_eq!(err.message(), "unique constraint violation");
        match &err {
            EFError::UniqueViolation { constraint, table } => {
                assert_eq!(constraint.as_deref(), Some("ux_order"));
                assert_eq!(table.as_deref(), Some("order_record"));
            }
            other => panic!("expected UniqueViolation, got {other:?}"),
        }
        let rendered = err.to_string();
        assert!(rendered.contains("order_record"), "got: {rendered}");
        assert!(rendered.contains("ux_order"), "got: {rendered}");
    }

    #[test]
    fn unique_violation_supports_missing_metadata() {
        let err = EFError::unique_violation(None, None);
        assert_eq!(err.code(), EFErrorCode::UniqueViolation);
        assert!(matches!(
            err,
            EFError::UniqueViolation {
                constraint: None,
                table: None
            }
        ));
    }

    #[test]
    fn with_source_keeps_unique_violation_variant() {
        let err = EFError::unique_violation(None, Some("t".into())).with_source("boom".into());
        assert_eq!(err.code(), EFErrorCode::UniqueViolation);
        assert!(matches!(
            err,
            EFError::UniqueViolation {
                table: Some(table),
                ..
            } if table == "t"
        ));
    }

    #[test]
    fn database_error_display_keeps_structured_diagnostics() {
        let info = DatabaseErrorInfo::new("duplicate key value violates unique constraint")
            .with_sqlstate("23505")
            .with_code("1062")
            .with_table("orders")
            .with_column("tenant_id")
            .with_constraint("ux_orders_tenant")
            .with_detail("Key (tenant_id)=(acme) already exists.")
            .with_hint("Use a different tenant id.")
            .with_sql("INSERT INTO orders (tenant_id) VALUES ($1)");
        let error = EFError::database(info);
        let rendered = error.to_string();
        for expected in [
            "23505",
            "1062",
            "orders",
            "tenant_id",
            "ux_orders_tenant",
            "already exists",
            "different tenant",
            "INSERT INTO orders",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected:?}: {rendered}"
            );
        }
        assert_eq!(
            error.message(),
            "duplicate key value violates unique constraint"
        );
        assert_eq!(error.code(), EFErrorCode::QuerySyntax);
    }

    #[test]
    fn database_error_code_classifies_transport_and_conversion() {
        let connection = EFError::database(
            DatabaseErrorInfo::new("server closed the connection").with_sqlstate("08006"),
        );
        assert_eq!(connection.code(), EFErrorCode::ConnectionRefused);
        let conversion = EFError::database(
            DatabaseErrorInfo::new("invalid input syntax for type bigint").with_sqlstate("22P02"),
        );
        assert_eq!(conversion.code(), EFErrorCode::TypeConversionFailed);
        let deadlock =
            EFError::database(DatabaseErrorInfo::new("deadlock detected").with_sqlstate("40P01"));
        assert_eq!(deadlock.code(), EFErrorCode::TransactionDeadlock);
    }

    #[test]
    fn database_error_sql_preview_is_bounded() {
        let huge = "x".repeat(MAX_SQL_PREVIEW_LEN + 100);
        let info = DatabaseErrorInfo::new("boom").with_sql(huge);
        let sql = info.sql.as_deref().unwrap();
        assert_eq!(sql.chars().count(), MAX_SQL_PREVIEW_LEN + 1);
        assert!(sql.ends_with('…'));
        // Empty SQL is not attached at all.
        assert!(DatabaseErrorInfo::new("boom").with_sql("   ").sql.is_none());
    }

    #[test]
    fn app_error_conversion_keeps_business_errors_and_hides_infrastructure() {
        use rust_core::AppError;

        let not_found: AppError = EFError::not_found("order 42 is missing").into();
        assert_eq!(not_found.status_code(), 404);
        assert_eq!(not_found.public_detail(), Some("order 42 is missing"));

        let validation: AppError = EFError::model_validation("amount must be positive").into();
        assert_eq!(validation.status_code(), 400);
        assert_eq!(validation.public_detail(), Some("amount must be positive"));

        let conflict: AppError =
            EFError::ConcurrencyConflict("version mismatch".into(), None).into();
        assert_eq!(conflict.status_code(), 409);
        assert_eq!(conflict.public_detail(), Some("version mismatch"));

        let unique: AppError = EFError::UniqueViolation {
            constraint: Some("ux_orders_number".into()),
            table: Some("orders".into()),
        }
        .into();
        assert_eq!(unique.status_code(), 409);
        assert_eq!(unique.public_detail(), Some(DETAIL_UNIQUE_VIOLATION));
        let operator = unique.operator().expect("operator detail");
        assert!(operator.contains("orders"), "{operator}");
        assert!(operator.contains("ux_orders_number"), "{operator}");

        for error in [
            EFError::query("SELECT * FROM secrets WHERE 1=0"),
            EFError::type_conversion("column `secret` is not an i64"),
        ] {
            let mapped: AppError = error.into();
            assert_eq!(mapped.status_code(), 400);
            assert_eq!(mapped.public_detail(), Some(DETAIL_INVALID_REQUEST));
            assert!(
                mapped
                    .operator()
                    .is_some_and(|detail| detail.contains("secret")),
                "the SQL/column detail must stay in the operator log"
            );
        }

        for error in [
            EFError::connection("pool closed"),
            EFError::Transaction("commit failed".into(), None),
            EFError::database(
                DatabaseErrorInfo::new("server closed the connection").with_sqlstate("08006"),
            ),
            EFError::database(DatabaseErrorInfo::new("deadlock detected").with_sqlstate("40P01")),
            EFError::database(DatabaseErrorInfo::new("busy").with_code("5")),
        ] {
            let mapped: AppError = error.into();
            assert_eq!(mapped.status_code(), 503);
            assert!(mapped.retryable());
            assert_eq!(mapped.public_detail(), Some(DETAIL_UNAVAILABLE));
        }

        for error in [
            EFError::database(
                DatabaseErrorInfo::new("relation does not exist").with_sqlstate("42P01"),
            ),
            EFError::migration("migration 0012 failed"),
            EFError::provider("provider not supported"),
            EFError::Configuration("missing password".into(), None),
            EFError::Other("boom".into(), None),
        ] {
            let mapped: AppError = error.into();
            assert_eq!(mapped.status_code(), 500);
            assert_eq!(mapped.public_detail(), None);
            assert!(mapped.operator().is_some(), "operator detail is required");
        }
    }

    #[test]
    fn transient_database_predicate_matches_documented_conditions() {
        for info in [
            DatabaseErrorInfo::new("").with_sqlstate("08006"),
            DatabaseErrorInfo::new("").with_sqlstate("40001"),
            DatabaseErrorInfo::new("").with_sqlstate("57P01"),
            DatabaseErrorInfo::new("").with_code("1205"),
            DatabaseErrorInfo::new("").with_code("1213"),
            DatabaseErrorInfo::new("").with_code("5"),
            DatabaseErrorInfo::new("").with_code("261"),
            DatabaseErrorInfo::new("").with_code("517"),
            DatabaseErrorInfo::new("").with_code("6"),
            DatabaseErrorInfo::new("").with_code("262"),
        ] {
            assert!(database_error_is_transient(&info), "{info:?}");
        }
        for info in [
            DatabaseErrorInfo::new("").with_sqlstate("23505"),
            DatabaseErrorInfo::new("").with_sqlstate("42P01"),
            DatabaseErrorInfo::new("").with_code("1"),
            DatabaseErrorInfo::new("").with_code("not-a-code"),
        ] {
            assert!(!database_error_is_transient(&info), "{info:?}");
        }
    }
}
