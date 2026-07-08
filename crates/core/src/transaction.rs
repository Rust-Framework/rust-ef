//! Transaction handle abstraction.
//!
//! Decouples transaction capabilities from `DbContext` so callers can hold a
//! typed handle (`Box<dyn ITransaction>`) returned by
//! `DbContext::begin_transaction()`. The handle exposes `commit` / `rollback`
//! (which consume it), savepoint operations, isolation level control, and a
//! `connection()` accessor used internally by `save_changes()` to reuse the
//! ambient transaction's connection.
//!
//! ## Two control modes
//!
//! - **Manual** — `let txn = ctx.begin_transaction().await?; ... txn.commit().await?;`
//!   The handle is returned without registering an ambient; `save_changes()`
//!   will self-manage its own transaction. Use this when you need the handle
//!   for explicit savepoint / isolation control.
//! - **Scoped** — `ctx.use_transaction(|ctx| Box::pin(async move { ... })).await?`
//!   Registers the handle as ambient; `save_changes()` calls inside the
//!   closure reuse the same transaction. Commits on `Ok`, rolls back on `Err`.

use crate::error::EFResult;
use crate::provider::{IAsyncConnection, IsolationLevel};
use std::future::Future;
use std::pin::Pin;

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
    /// ambient transaction's connection.
    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send);
}

/// Default `ITransaction` implementation wrapping an `IAsyncConnection`.
///
/// The connection is held in `Option` so `commit` / `rollback` can take it
/// out by consuming the handle; subsequent savepoint/isolation calls on a
/// consumed handle return `EFError::Transaction`.
pub struct DbTransaction {
    conn: Option<Box<dyn IAsyncConnection>>,
}

impl DbTransaction {
    pub fn new(conn: Box<dyn IAsyncConnection>) -> Self {
        Self { conn: Some(conn) }
    }
}

impl ITransaction for DbTransaction {
    fn commit(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>> {
        Box::pin(async move {
            let mut conn = self.conn.expect("transaction already consumed");
            conn.commit_transaction().await
        })
    }

    fn rollback(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>> {
        Box::pin(async move {
            let mut conn = self.conn.expect("transaction already consumed");
            conn.rollback_transaction().await
        })
    }

    fn create_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::transaction("transaction consumed".into()))?
                .create_savepoint(name)
                .await
        })
    }

    fn release_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::transaction("transaction consumed".into()))?
                .release_savepoint(name)
                .await
        })
    }

    fn rollback_point<'a>(
        &'a mut self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::transaction("transaction consumed".into()))?
                .rollback_to_savepoint(name)
                .await
        })
    }

    fn set_isolation<'a>(
        &'a mut self,
        level: IsolationLevel,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::transaction("transaction consumed".into()))?
                .set_transaction_isolation(level)
                .await
        })
    }

    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send) {
        self.conn
            .as_mut()
            .expect("transaction already consumed")
            .as_mut()
    }
}
