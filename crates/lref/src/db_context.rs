//! DbContext trait and ChangeTracker — the session / unit-of-work layer.
//!
//! Provides `save_changes_all!()` macro for batch entity saving and
//! the standalone `save_one_set()` function for single-type saves.

use crate::entity::{EntitySnapshot, EntityType, FromRow, GetKeyValues};
use crate::error::LrefResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{AsyncConnection, DatabaseProvider};
use crate::change_executor::ChangeExecutor;
use crate::db_set::DbSet;
use crate::tracking::ChangeTracker;

/// The DbContext trait represents a session with the database.
#[async_trait::async_trait]
pub trait DbContext: Send + Sync + Sized {
    type Provider: crate::provider::DatabaseProvider;
    fn provider(&self) -> &Self::Provider;
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker;
    fn change_tracker(&self) -> &ChangeTracker;
    async fn save_changes(&mut self) -> LrefResult<SaveChangesResult>;

    async fn begin_transaction(&self) -> LrefResult<Box<dyn AsyncConnection>> {
        let mut conn: Box<dyn AsyncConnection> = self.provider().get_connection().await?;
        conn.begin_transaction().await?;
        Ok(conn)
    }

    async fn use_transaction<F, Fut, R>(&self, f: F) -> LrefResult<R>
    where
        F: FnOnce(&mut dyn AsyncConnection) -> Fut + Send,
        Fut: std::future::Future<Output = LrefResult<R>> + Send,
        R: Send,
    {
        let mut conn: Box<dyn AsyncConnection> = self.provider().get_connection().await?;
        conn.begin_transaction().await?;
        match f(&mut *conn).await {
            Ok(result) => {
                conn.commit_transaction().await?;
                Ok(result)
            }
            Err(e) => {
                let _ = conn.rollback_transaction().await;
                Err(e)
            }
        }
    }

    fn set_logging(&mut self, _enabled: bool) {}
    fn is_logging_enabled(&self) -> bool { false }
    #[allow(unused_variables)]
    fn log_sql(&self, sql: &str, params_count: usize) {}

    /// Creates all tables in the database based on registered entity metadata.
    async fn ensure_created(&self) -> LrefResult<()> {
        let conn_str = format!("{}", self.provider().name());
        let _ = conn_str;
        Err(crate::error::LrefError::Configuration(
            "ensure_created requires entity metadata. Use migration engine instead.".into()
        ))
    }

    /// Drops all tables in the database.
    async fn ensure_deleted(&self) -> LrefResult<()> {
        Err(crate::error::LrefError::Configuration(
            "ensure_deleted requires entity metadata. Use migration engine instead.".into()
        ))
    }
}

/// Saves changes for one entity type within an active transaction.
/// Used by `save_changes_all!` macro. Can also be called directly.
pub async fn save_one_set<E>(
    conn: &mut dyn AsyncConnection,
    provider: &dyn DatabaseProvider,
    db_set: &mut DbSet<E>,
) -> LrefResult<(usize, usize, usize)>
where
    E: EntityType + EntitySnapshot + GetKeyValues + FromRow,
{
    let meta = E::entity_meta();

    let added: Vec<(&E, &EntityTypeMeta)> = db_set.added_entities().into_iter().map(|e| (e, &meta)).collect();
    let modified: Vec<(&E, &EntityTypeMeta)> = db_set.modified_entities().into_iter().map(|e| (e, &meta)).collect();
    let deleted: Vec<(&E, &EntityTypeMeta)> = db_set.deleted_entities().into_iter().map(|e| (e, &meta)).collect();

    let mut added_count = 0usize;
    let mut updated_count = 0usize;
    let mut deleted_count = 0usize;

    if !added.is_empty() {
        added_count = ChangeExecutor::execute_inserts(conn, provider, &added, |_, _| {}).await?;
    }
    if !modified.is_empty() {
        updated_count = ChangeExecutor::execute_updates(conn, provider, &modified).await?;
    }
    if !deleted.is_empty() {
        deleted_count = ChangeExecutor::execute_deletes(conn, provider, &deleted).await?;
    }

    Ok((added_count, updated_count, deleted_count))
}

/// Macro to save changes for multiple entity types in a single transaction.
#[macro_export]
macro_rules! save_changes_all {
    ($ctx:expr, $first:expr $(, $rest:expr)* $(,)?) => {{
        $ctx.change_tracker_mut().detect_changes();
        let mut conn = $ctx.provider().get_connection().await?;
        conn.begin_transaction().await?;

        let mut added = 0usize;
        let mut updated = 0usize;
        let mut deleted = 0usize;

        {
            let (a, u, d) = $crate::db_context::save_one_set(
                &mut *conn, $ctx.provider(), &mut $first
            ).await?;
            added += a; updated += u; deleted += d;
        }
        $(
            {
                let (a, u, d) = $crate::db_context::save_one_set(
                    &mut *conn, $ctx.provider(), &mut $rest
                ).await?;
                added += a; updated += u; deleted += d;
            }
        )*

        conn.commit_transaction().await?;
        $ctx.change_tracker_mut().accept_all_changes();
        $first.clear_entries();
        $($rest.clear_entries();)*

        Ok($crate::db_context::SaveChangesResult { added, updated, deleted })
    }};
}

/// Result of calling save_changes().
#[derive(Debug, Clone)]
pub struct SaveChangesResult {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
}

impl SaveChangesResult {
    pub fn total(&self) -> usize {
        self.added + self.updated + self.deleted
    }
}

impl std::fmt::Display for SaveChangesResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} entities modified ({} added, {} updated, {} deleted)",
            self.total(),
            self.added,
            self.updated,
            self.deleted
        )
    }
}
