#[cfg(feature = "tracing")]
use std::time::Instant;

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
