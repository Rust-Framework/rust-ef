//! Error types for Rust Entity Framework (rust-ef).
//!
//! Each variant carries an optional `#[source]` for preserving the original
//! error chain, enabling programmatic inspection via `std::error::Error::source()`.
//! Convenience constructors (`EFError::query(msg)`, etc.) keep call sites concise;
//! `with_source` and `with_sql` builders attach context after construction.

use thiserror::Error;

/// Boxed error suitable for `EFError::source`.
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Represents all possible errors that can occur in rust-ef operations.
#[derive(Error, Debug)]
pub enum EFError {
    /// Database connection error.
    #[error("database connection error: {0}")]
    Connection(String, #[source] Option<BoxError>),

    /// Query execution error.
    #[error("query error: {0}")]
    Query(String, #[source] Option<BoxError>),

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
}
