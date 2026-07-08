#[cfg(feature = "tracing")]
use std::time::Instant;

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
