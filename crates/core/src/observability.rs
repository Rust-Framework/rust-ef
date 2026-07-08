//! Tracing/observability instrumentation guards.
//!
//! When the `tracing` feature is disabled, all guards are zero-sized types
//! with no-op constructors — eliminated entirely by the compiler.
//!
//! # Usage
//!
//! Providers and connections create guards at the start of instrumented
//! operations. The guard emits a tracing event on `Drop`:
//!
//! - [`QueryGuard`] — wraps a single `query`/`execute` call; emits `DEBUG`
//!   on completion, `WARN` if the elapsed time exceeds the slow-query
//!   threshold.
//! - [`PoolAcquireGuard`] — wraps a connection-pool acquisition; emits
//!   `INFO` with the acquire duration.
//! - [`SaveChangesGuard`] — wraps a `save_changes` call; emits `INFO`
//!   with the total elapsed time.

use std::time::Duration;
#[cfg(feature = "tracing")]
use std::time::Instant;

// ==================== QueryGuard ====================

/// Guards a query execution span, emitting timing events on drop.
///
/// When `tracing` is enabled, logs query start (`DEBUG`) and completion
/// (`DEBUG`, or `WARN` if slow). When disabled, it is a ZST no-op.
#[cfg(feature = "tracing")]
pub struct QueryGuard {
    sql: String,
    start: Instant,
    threshold: Option<Duration>,
}

#[cfg(feature = "tracing")]
impl QueryGuard {
    pub fn new(sql: &str, threshold: Option<Duration>) -> Self {
        tracing::debug!(target: "rust_ef::query", sql = %sql, "query started");
        Self {
            sql: sql.to_string(),
            start: Instant::now(),
            threshold,
        }
    }
}

#[cfg(feature = "tracing")]
impl Drop for QueryGuard {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        let elapsed_ms = elapsed.as_millis() as u64;
        if let Some(threshold) = self.threshold {
            if elapsed >= threshold {
                tracing::warn!(
                    target: "rust_ef::query",
                    sql = %self.sql,
                    elapsed_ms,
                    threshold_ms = threshold.as_millis() as u64,
                    "slow query detected"
                );
                return;
            }
        }
        tracing::debug!(
            target: "rust_ef::query",
            sql = %self.sql,
            elapsed_ms,
            "query completed"
        );
    }
}

/// No-op stub when tracing is disabled — zero-sized, eliminated by compiler.
#[cfg(not(feature = "tracing"))]
pub struct QueryGuard;

#[cfg(not(feature = "tracing"))]
impl QueryGuard {
    pub fn new(_sql: &str, _threshold: Option<Duration>) -> Self {
        Self
    }
}

// ==================== PoolAcquireGuard ====================

/// Guards a connection pool acquisition, emitting acquire timing on drop.
#[cfg(feature = "tracing")]
pub struct PoolAcquireGuard {
    provider: &'static str,
    start: Instant,
}

#[cfg(feature = "tracing")]
impl PoolAcquireGuard {
    pub fn new(provider: &'static str) -> Self {
        Self {
            provider,
            start: Instant::now(),
        }
    }
}

#[cfg(feature = "tracing")]
impl Drop for PoolAcquireGuard {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        tracing::info!(
            target: "rust_ef::pool",
            provider = self.provider,
            acquire_ms = elapsed.as_millis() as u64,
            "connection acquired"
        );
    }
}

#[cfg(not(feature = "tracing"))]
pub struct PoolAcquireGuard;

#[cfg(not(feature = "tracing"))]
impl PoolAcquireGuard {
    pub fn new(_provider: &'static str) -> Self {
        Self
    }
}

// ==================== SaveChangesGuard ====================

/// Guards a `save_changes` operation, emitting timing on drop.
#[cfg(feature = "tracing")]
pub struct SaveChangesGuard {
    start: Instant,
}

#[cfg(feature = "tracing")]
impl SaveChangesGuard {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

#[cfg(feature = "tracing")]
impl Default for SaveChangesGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "tracing")]
impl Drop for SaveChangesGuard {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        tracing::info!(
            target: "rust_ef::save_changes",
            elapsed_ms = elapsed.as_millis() as u64,
            "save_changes completed"
        );
    }
}

#[cfg(not(feature = "tracing"))]
pub struct SaveChangesGuard;

#[cfg(not(feature = "tracing"))]
impl Default for SaveChangesGuard {
    fn default() -> Self {
        Self
    }
}

#[cfg(not(feature = "tracing"))]
impl SaveChangesGuard {
    pub fn new() -> Self {
        Self
    }
}
