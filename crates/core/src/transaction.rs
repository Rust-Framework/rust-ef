//! Transaction handle abstraction.
//!
//! `DbContext::begin_transaction()` returns a context-scoped
//! [`ITransaction`]. The handle exposes `commit` / `rollback` (which consume
//! it), savepoint operations, isolation control, and a `connection()` accessor
//! used internally by `save_changes()`.
//!
//! ## Control modes
//!
//! - **Manual** — `let txn = ctx.begin_transaction().await?; ... txn.commit().await?;`
//!   The transaction is registered on the context: queries, bulk DML, raw SQL
//!   and `save_changes()` all use its connection. Commit or rollback explicitly.
//! - **Closure** — `ctx.use_transaction(|ctx| Box::pin(async move { ... })).await?`.
//!   Commits on `Ok`, rolls back on `Err`.
//!
//! Any handle dropped without `commit` / `rollback` is rolled back on a
//! background task, matching EF Core / ADO.NET `Dispose` semantics. The
//! runtime is captured when the handle is created, so dropping it outside a
//! Tokio context can still schedule the rollback. The rollback is best effort:
//! it cannot run if the runtime is already shut down, and `panic = "abort"`
//! skips `Drop` entirely.

use crate::error::{EFError, EFResult};
use crate::provider::{IAsyncConnection, IsolationLevel};
use crate::session::{Session, SessionConnection, SharedTxn};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Transaction handle abstracting the connection-level transaction methods.
///
/// `commit` / `rollback` consume `self: Box<Self>` so use-after-commit is
/// impossible at the type level. Savepoint and isolation methods borrow the
/// handle.
pub trait ITransaction: Send + Sync {
    /// Commits the transaction and consumes the handle.
    fn commit(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>>;

    /// Rolls back the transaction and consumes the handle.
    fn rollback(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>>;

    /// Creates a savepoint within the current transaction.
    fn create_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>>;

    /// Releases (commits) a previously created savepoint.
    fn release_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>>;

    /// Rolls back to the named savepoint, preserving the outer transaction.
    fn rollback_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>>;

    /// Sets the isolation level of the current transaction.
    fn set_isolation<'a>(
        &'a mut self,
        level: IsolationLevel,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>>;

    /// Exposes the underlying connection so `save_changes()` can reuse the
    /// transaction's connection.
    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send);
}

/// A connection shared with the `DbContext` session.
struct TxnState {
    shared: Arc<SharedTxn>,
    facade: SessionConnection,
}

/// Default `ITransaction` implementation for context-scoped transactions.
pub struct DbTransaction {
    state: Option<TxnState>,
    /// Commit / rollback / drop clears the context's active transaction so
    /// subsequent statements fall back to the pool.
    session: Arc<Session>,
    /// Runtime captured at begin time, so dropping the handle outside a Tokio
    /// context can still schedule the rollback.
    runtime: Option<tokio::runtime::Handle>,
}

impl DbTransaction {
    pub(crate) fn new_shared(
        shared: Arc<SharedTxn>,
        facade: SessionConnection,
        session: Arc<Session>,
    ) -> Self {
        Self {
            state: Some(TxnState { shared, facade }),
            session,
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }
}

impl ITransaction for DbTransaction {
    fn commit(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>> {
        Box::pin(async move {
            let mut this = self;
            let Some(state) = this.state.take() else {
                return Err(EFError::transaction("transaction already consumed"));
            };
            let result = match state.shared.take().await {
                Ok(mut conn) => match conn.commit_transaction().await {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        // Best-effort cleanup if COMMIT failed; the connection
                        // may be in an unknown state either way.
                        let _ = conn.rollback_transaction().await;
                        Err(e)
                    }
                },
                Err(e) => Err(e),
            };
            this.session.clear().await;
            result
        })
    }

    fn rollback(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>> {
        Box::pin(async move {
            let mut this = self;
            let Some(state) = this.state.take() else {
                return Err(EFError::transaction("transaction already consumed"));
            };
            let result = match state.shared.take().await {
                Ok(mut conn) => conn.rollback_transaction().await,
                Err(e) => Err(e),
            };
            this.session.clear().await;
            result
        })
    }

    fn create_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?
                .facade
                .create_savepoint(name)
                .await
        })
    }

    fn release_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?
                .facade
                .release_savepoint(name)
                .await
        })
    }

    fn rollback_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?
                .facade
                .rollback_to_savepoint(name)
                .await
        })
    }

    fn set_isolation<'a>(
        &'a mut self,
        level: IsolationLevel,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?
                .facade
                .set_transaction_isolation(level)
                .await
        })
    }

    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send) {
        &mut self
            .state
            .as_mut()
            .expect("transaction already consumed")
            .facade
    }
}

impl Drop for DbTransaction {
    fn drop(&mut self) {
        let state = self.state.take();
        self.session.clear_now();
        let Some(state) = state else {
            return;
        };
        let handle = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok());
        let Some(handle) = handle else {
            return;
        };
        let session = Arc::clone(&self.session);
        handle.spawn(async move {
            if let Ok(mut conn) = state.shared.take().await {
                let _ = conn.rollback_transaction().await;
            }
            session.clear().await;
        });
    }
}
