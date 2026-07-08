use std::time::Duration;
#[cfg(feature = "tracing")]
use std::time::Instant;

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
