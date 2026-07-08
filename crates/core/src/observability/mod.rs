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

mod pool_guard;
mod query_guard;
mod save_changes_guard;

pub use pool_guard::PoolAcquireGuard;
pub use query_guard::QueryGuard;
pub use save_changes_guard::SaveChangesGuard;
