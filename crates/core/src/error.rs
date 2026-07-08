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

    /// Type conversion error.
    #[error("type conversion error: {0}")]
    TypeConversion(String, #[source] Option<BoxError>),

    /// General / unknown error.
    #[error("{0}")]
    Other(String, #[source] Option<BoxError>),
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

    pub fn type_conversion(msg: impl Into<String>) -> Self {
        EFError::TypeConversion(msg.into(), None)
    }

    pub fn other(msg: impl Into<String>) -> Self {
        EFError::Other(msg.into(), None)
    }

    /// Attaches a source error to this `EFError`, preserving the error chain.
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
            EFError::TypeConversion(m, _) => m,
            EFError::Other(m, _) => m,
        }
    }
}

/// Result type alias for rust-ef operations.
pub type EFResult<T> = Result<T, EFError>;
