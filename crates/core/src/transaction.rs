//! Transaction handle abstraction.
//!
//! Decouples transaction capabilities from `DbContext` so callers can hold a
//! typed handle (`Box<dyn ITransaction>`) returned by
//! `DbContext::begin_transaction()`. The handle exposes `commit` / `rollback`
//! (which consume it), savepoint operations, isolation level control, and a
//! `connection()` accessor used internally by `save_changes()` to reuse the
//! transaction's connection.
//!
//! ## Control modes
//!
//! - **Scoped** — `ctx.use_transaction(|ctx| Box::pin(async move { ... })).await?`
//!   or `ctx.transaction(...)`. Registers the handle as ambient; every
//!   statement issued through the context uses the transaction's connection.
//!   Commits on `Ok`, rolls back on `Err` / `TxOutcome::Rollback`.
//! - **Manual** — `let txn = ctx.begin_transaction().await?; ... txn.commit().await?;`
//!   Returns a **standalone** handle. It is not registered as ambient, so
//!   queries and `save_changes()` do not join it; use it for raw SQL and
//!   explicit savepoint/isolation control. Use `ctx.transaction(...)` when the
//!   whole context should join the transaction.
//!
//! Any transaction handle that is dropped without `commit` / `rollback` is
//! rolled back on a background task, matching the `Dispose`-rolls-back
//! convention of EF Core / ADO.NET. The rollback is best effort: it requires a
//! Tokio runtime to be available when the handle is dropped.

use crate::error::{EFError, EFResult};
use crate::provider::{IAsyncConnection, IsolationLevel};
use crate::session::{Session, SessionConnection, SharedTxn};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// The outcome of a [`DbContext::transaction`](crate::db_context::DbContext::transaction)
/// closure.
///
/// Unlike [`DbContext::use_transaction`](crate::db_context::DbContext::use_transaction),
/// which commits on `Ok` and rolls back only on `Err`, this type lets a handler
/// abort the database work and still return a successful business result.
///
/// ```rust,ignore
/// let result = ctx.transaction(|ctx| Box::pin(async move {
///     // ... database work ...
///     if !meets_condition {
///         return Ok(TxOutcome::Rollback(BizResult::Rejected));
///     }
///     Ok(TxOutcome::Commit(BizResult::Done))
/// })).await?;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxOutcome<T> {
    /// Commit the transaction and return the value.
    Commit(T),
    /// Roll the transaction back, detach tracked entities, and return the value.
    Rollback(T),
}

/// Transaction handle abstracting the Priority 2 connection-level transaction
/// methods.
///
/// `commit` / `rollback` consume `self: Box<Self>` to make use-after-commit
/// impossible at the type level. Savepoint and isolation methods take
/// `&mut self` and return a boxed future borrowing the handle.
///
/// `Send + Sync` is required so that `Box<dyn ITransaction>` can be stored in
/// `DbContext` (which DI registers as `Scoped`, requiring `Send + Sync`).
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

    /// Whether this transaction is scoped to the `DbContext`.
    ///
    /// Scoped transactions (`use_transaction` / `transaction`) route queries
    /// and bulk DML through the transaction connection and get per-save
    /// savepoints. A manual [`DbContext::begin_transaction`](crate::db_context::DbContext::begin_transaction)
    /// handle is standalone and keeps the legacy behavior.
    fn is_scoped(&self) -> bool {
        false
    }
}

/// Where the transaction's connection lives.
enum TxnState {
    /// A connection owned outright by the handle (standalone
    /// `DbTransaction::new`).
    Owned(Option<Box<dyn IAsyncConnection>>),
    /// A connection shared with the `DbContext` session, so queries and bulk
    /// DML inside the transaction observe the same connection.
    Shared {
        shared: Arc<SharedTxn>,
        facade: SessionConnection,
    },
}

/// Default `ITransaction` implementation.
///
/// In `Owned` mode the connection is held in `Option` so `commit` / `rollback`
/// can take it out by consuming the handle. In `Shared` mode the connection
/// lives in the session slot and is taken out on commit / rollback.
pub struct DbTransaction {
    state: Option<TxnState>,
    /// When set, commit / rollback / drop clears the context's active
    /// transaction so subsequent statements fall back to the pool.
    session: Option<Arc<Session>>,
}

impl DbTransaction {
    pub fn new(conn: Box<dyn IAsyncConnection>) -> Self {
        Self {
            state: Some(TxnState::Owned(Some(conn))),
            session: None,
        }
    }

    pub(crate) fn new_shared(
        shared: Arc<SharedTxn>,
        facade: SessionConnection,
        session: Option<Arc<Session>>,
    ) -> Self {
        Self {
            state: Some(TxnState::Shared { shared, facade }),
            session,
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
            let result = match state {
                TxnState::Owned(conn) => match conn {
                    Some(mut conn) => conn.commit_transaction().await,
                    None => Err(EFError::transaction("transaction already consumed")),
                },
                TxnState::Shared { shared, .. } => match shared.take().await {
                    Ok(mut conn) => conn.commit_transaction().await,
                    Err(e) => Err(e),
                },
            };
            if let Some(session) = this.session.as_ref() {
                session.clear().await;
            }
            result
        })
    }

    fn rollback(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>> {
        Box::pin(async move {
            let mut this = self;
            let Some(state) = this.state.take() else {
                return Err(EFError::transaction("transaction already consumed"));
            };
            let result = match state {
                TxnState::Owned(conn) => match conn {
                    Some(mut conn) => conn.rollback_transaction().await,
                    None => Err(EFError::transaction("transaction already consumed")),
                },
                TxnState::Shared { shared, .. } => match shared.take().await {
                    Ok(mut conn) => conn.rollback_transaction().await,
                    Err(e) => Err(e),
                },
            };
            if let Some(session) = this.session.as_ref() {
                session.clear().await;
            }
            result
        })
    }

    fn create_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let state = self
                .state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?;
            match state {
                TxnState::Owned(conn) => {
                    conn.as_mut()
                        .ok_or_else(|| EFError::transaction("transaction consumed"))?
                        .create_savepoint(name)
                        .await
                }
                TxnState::Shared { facade, .. } => facade.create_savepoint(name).await,
            }
        })
    }

    fn release_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let state = self
                .state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?;
            match state {
                TxnState::Owned(conn) => {
                    conn.as_mut()
                        .ok_or_else(|| EFError::transaction("transaction consumed"))?
                        .release_savepoint(name)
                        .await
                }
                TxnState::Shared { facade, .. } => facade.release_savepoint(name).await,
            }
        })
    }

    fn rollback_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let state = self
                .state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?;
            match state {
                TxnState::Owned(conn) => {
                    conn.as_mut()
                        .ok_or_else(|| EFError::transaction("transaction consumed"))?
                        .rollback_to_savepoint(name)
                        .await
                }
                TxnState::Shared { facade, .. } => facade.rollback_to_savepoint(name).await,
            }
        })
    }

    fn set_isolation<'a>(
        &'a mut self,
        level: IsolationLevel,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let state = self
                .state
                .as_mut()
                .ok_or_else(|| EFError::transaction("transaction already consumed"))?;
            match state {
                TxnState::Owned(conn) => {
                    conn.as_mut()
                        .ok_or_else(|| EFError::transaction("transaction consumed"))?
                        .set_transaction_isolation(level)
                        .await
                }
                TxnState::Shared { facade, .. } => facade.set_transaction_isolation(level).await,
            }
        })
    }

    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send) {
        match self.state.as_mut().expect("transaction already consumed") {
            TxnState::Owned(conn) => conn
                .as_mut()
                .expect("transaction already consumed")
                .as_mut(),
            TxnState::Shared { facade, .. } => facade,
        }
    }

    fn is_scoped(&self) -> bool {
        matches!(&self.state, Some(TxnState::Shared { .. }))
    }
}

impl Drop for DbTransaction {
    fn drop(&mut self) {
        let state = self.state.take();
        let session = self.session.take();
        if let Some(session) = &session {
            // Detach before the background rollback so new statements do not
            // join a transaction that is about to be rolled back. This also
            // covers the cancellation window where commit/rollback already
            // took the connection out of the handle.
            session.clear_now();
        }
        let Some(state) = state else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            match state {
                TxnState::Owned(Some(mut conn)) => {
                    let _ = conn.rollback_transaction().await;
                }
                TxnState::Owned(None) => {}
                TxnState::Shared { shared, .. } => {
                    if let Ok(mut conn) = shared.take().await {
                        let _ = conn.rollback_transaction().await;
                    }
                }
            }
            if let Some(session) = session {
                session.clear().await;
            }
        });
    }
}
