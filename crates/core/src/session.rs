//! Per-`DbContext` connection session.
//!
//! `DbContext` wraps its (pooled) write provider in a [`SessionProvider`], so
//! every statement issued through the context — queries, bulk DML, raw SQL and
//! `save_changes` — borrows the same connection while an explicit transaction
//! is open. Before this module existed, `use_transaction` only covered the DML
//! executed by `save_changes`: queries and `execute_update` / `execute_delete`
//! acquired a different pooled connection, so they could not see uncommitted
//! writes and committed immediately outside the transaction.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::error::{EFError, EFResult};
use crate::migration::MigrationDialect;
use crate::provider::{
    DbValue, IAsyncConnection, IDatabaseProvider, ISqlGenerator, IsolationLevel,
};

/// The connection slot shared by the transaction handle and every
/// [`SessionConnection`] clone.
///
/// The connection is taken out once the transaction is committed or rolled
/// back; afterwards every attempt to run a statement on the session reports
/// that the transaction is already consumed.
pub(crate) struct SharedTxn {
    conn: Mutex<Option<Box<dyn IAsyncConnection>>>,
}

impl SharedTxn {
    fn new(conn: Box<dyn IAsyncConnection>) -> Self {
        Self {
            conn: Mutex::new(Some(conn)),
        }
    }

    /// Takes the underlying connection out of the slot. Used by
    /// `DbTransaction::commit` / `rollback`.
    pub(crate) async fn take(&self) -> EFResult<Box<dyn IAsyncConnection>> {
        self.conn
            .lock()
            .await
            .take()
            .ok_or_else(|| EFError::transaction("transaction connection is already consumed"))
    }

    /// Synchronously takes the connection when no statement is currently
    /// using it. Used by `DbTransaction::Drop`.
    pub(crate) fn try_take(&self) -> Option<Box<dyn IAsyncConnection>> {
        self.conn.try_lock().ok()?.take()
    }
}

impl Drop for SharedTxn {
    fn drop(&mut self) {
        // Last owner is gone while a connection is still attached: no one can
        // commit it anymore. Discard/close it so the database rolls back.
        if let Ok(mut guard) = self.conn.try_lock() {
            if let Some(conn) = guard.take() {
                conn.discard();
            }
        }
    }
}

/// The session state owned by one `DbContext`.
pub(crate) struct Session {
    write: Arc<dyn IDatabaseProvider>,
    txn: Mutex<Option<Arc<SharedTxn>>>,
}

impl Session {
    pub(crate) fn new(write: Arc<dyn IDatabaseProvider>) -> Self {
        Self {
            write,
            txn: Mutex::new(None),
        }
    }

    /// Opens a connection from the write pool, begins a transaction on it and
    /// registers it as the session's active transaction. Applies the requested
    /// isolation level at transaction start (providers that need it before
    /// `BEGIN` handle it in `begin_transaction_with`).
    pub(crate) async fn begin_with(
        &self,
        isolation: Option<IsolationLevel>,
    ) -> EFResult<Arc<SharedTxn>> {
        let mut guard = self.txn.lock().await;
        if guard.is_some() {
            return Err(EFError::transaction(
                "a transaction is already active on this DbContext",
            ));
        }
        let mut conn = self.write.get_connection().await?;
        match isolation {
            Some(level) => conn.begin_transaction_with(level).await?,
            None => conn.begin_transaction().await?,
        }
        let shared = Arc::new(SharedTxn::new(conn));
        *guard = Some(Arc::clone(&shared));
        Ok(shared)
    }

    pub(crate) async fn clear(&self) {
        *self.txn.lock().await = None;
    }

    pub(crate) async fn is_active(&self) -> bool {
        self.txn.lock().await.is_some()
    }

    /// Best-effort synchronous clear, used from `Drop` to detach the doomed
    /// transaction before the background rollback runs.
    pub(crate) fn clear_now(&self) {
        if let Ok(mut guard) = self.txn.try_lock() {
            *guard = None;
        }
    }

    /// Returns a connection for a single statement.
    ///
    /// While a transaction is active this is a handle onto the transaction's
    /// connection; otherwise it is a fresh connection from the write pool.
    pub(crate) async fn get_connection(&self) -> EFResult<Box<dyn IAsyncConnection>> {
        let shared = self.txn.lock().await.clone();
        match shared {
            Some(shared) => Ok(Box::new(SessionConnection::new(shared))),
            None => self.write.get_connection().await,
        }
    }
}

/// A single-statement handle onto the session's active transaction connection.
///
/// `IAsyncConnection` takes `&mut self` per statement, so handing out a
/// long-lived `&mut` borrow is not possible. This handle locks the shared slot
/// for the duration of one statement and is cheap to clone per statement.
pub(crate) struct SessionConnection {
    shared: Arc<SharedTxn>,
}

impl SessionConnection {
    pub(crate) fn new(shared: Arc<SharedTxn>) -> Self {
        Self { shared }
    }
}

#[async_trait]
impl IAsyncConnection for SessionConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        let mut guard = self.shared.conn.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| EFError::transaction("transaction connection is already consumed"))?;
        conn.execute(sql, params).await
    }

    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<DbValue>>> {
        let mut guard = self.shared.conn.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| EFError::transaction("transaction connection is already consumed"))?;
        conn.query(sql, params).await
    }

    async fn begin_transaction(&mut self) -> EFResult<()> {
        Err(EFError::transaction(
            "cannot begin a nested transaction on an active transaction connection",
        ))
    }

    async fn commit_transaction(&mut self) -> EFResult<()> {
        Err(EFError::transaction(
            "use the transaction handle to commit an active transaction",
        ))
    }

    async fn rollback_transaction(&mut self) -> EFResult<()> {
        Err(EFError::transaction(
            "use the transaction handle to roll back an active transaction",
        ))
    }

    async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
        let mut guard = self.shared.conn.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| EFError::transaction("transaction connection is already consumed"))?;
        conn.create_savepoint(name).await
    }

    async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
        let mut guard = self.shared.conn.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| EFError::transaction("transaction connection is already consumed"))?;
        conn.release_savepoint(name).await
    }

    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
        let mut guard = self.shared.conn.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| EFError::transaction("transaction connection is already consumed"))?;
        conn.rollback_to_savepoint(name).await
    }

    async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
        let mut guard = self.shared.conn.lock().await;
        let conn = guard
            .as_mut()
            .ok_or_else(|| EFError::transaction("transaction connection is already consumed"))?;
        conn.set_transaction_isolation(level).await
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&mut self, _threshold: std::time::Duration) {
        // The pooled connection already received the provider's threshold when
        // it was acquired; per-session overrides are not needed here.
    }

    fn discard(self: Box<Self>) {
        if let Some(conn) = self.shared.try_take() {
            conn.discard();
        }
    }
}

/// `IDatabaseProvider` facade that routes every connection request through the
/// owning `DbContext`'s [`Session`].
pub(crate) struct SessionProvider {
    session: Arc<Session>,
}

impl SessionProvider {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl IDatabaseProvider for SessionProvider {
    fn sql_generator(&self) -> &'static dyn ISqlGenerator {
        self.session.write.sql_generator()
    }

    async fn get_connection(&self) -> EFResult<Box<dyn IAsyncConnection>> {
        self.session.get_connection().await
    }

    async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
        self.session.write.execute_migration_command(sql).await
    }

    fn name(&self) -> &str {
        self.session.write.name()
    }

    fn migration_dialect(&self) -> MigrationDialect {
        self.session.write.migration_dialect()
    }

    #[cfg(feature = "tracing")]
    fn set_slow_query_threshold(&self, threshold: std::time::Duration) {
        self.session.write.set_slow_query_threshold(threshold);
    }
}
