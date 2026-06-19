//! Error types for Rust Entity Framework (rust-ef).

use thiserror::Error;

/// Represents all possible errors that can occur in lref operations.
#[derive(Error, Debug)]
pub enum LrefError {
    /// Database connection error.
    #[error("database connection error: {0}")]
    Connection(String),

    /// Query execution error.
    #[error("query error: {0}")]
    Query(String),

    /// Entity not found error.
    #[error("entity not found: {0}")]
    NotFound(String),

    /// Model validation error.
    #[error("model validation error: {0}")]
    ModelValidation(String),

    /// Migration error.
    #[error("migration error: {0}")]
    Migration(String),

    /// Provider-specific error.
    #[error("provider error: {0}")]
    Provider(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Configuration(String),

    /// Change tracking error.
    #[error("change tracking error: {0}")]
    ChangeTracking(String),

    /// Transaction error.
    #[error("transaction error: {0}")]
    Transaction(String),

    /// Concurrency conflict error.
    #[error("concurrency conflict: {0}")]
    ConcurrencyConflict(String),

    /// Type conversion error.
    #[error("type conversion error: {0}")]
    TypeConversion(String),

    /// General / unknown error.
    #[error("{0}")]
    Other(String),
}

/// Result type alias for lref operations.
pub type LrefResult<T> = Result<T, LrefError>;
