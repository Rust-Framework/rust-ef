//! Applying a runtime [`MigrationDiff`] to a live database (EF-9).
//!
//! Kept in a separate module from `engine_ops.rs` so the runtime upgrade path is
//! self-contained: the versioned-migration workflow (`apply`/`revert`) is not
//! touched.
//!
//! # Atomicity matrix
//!
//! | Dialect | Behavior |
//! |---|---|
//! | SQLite | One transaction. Foreign-key enforcement is disabled *outside* the transaction (otherwise `DROP TABLE` would fire `ON DELETE` actions on child rows), `PRAGMA foreign_key_check` runs before commit, any error rolls everything back, and the connection's original PRAGMA value is restored. |
//! | PostgreSQL | One transaction; any failed statement rolls the whole plan back. |
//! | MySQL | DDL implicitly commits, so statements are executed individually. Partial application is possible (engine limitation, see EF-13); the plan is validated by `diff` beforehand. |

use crate::error::{EFError, EFResult};
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};

use super::dialect::MigrationDialect;
use super::engine::MigrationEngine;
use super::live_diff::MigrationDiff;

/// Outcome of [`MigrationEngine::apply_diff`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffApplyResult {
    /// Number of statements executed.
    pub executed_statements: usize,
    /// Tables that were rebuilt (SQLite).
    pub rebuilt_tables: Vec<String>,
}

impl DiffApplyResult {
    /// Whether nothing had to be applied.
    pub fn is_noop(&self) -> bool {
        self.executed_statements == 0
    }
}

impl MigrationEngine {
    /// Applies a runtime model diff to a live database.
    ///
    /// The plan is expected to come from [`Self::diff`] computed against an
    /// up-to-date introspection of the same database. No migration history row
    /// is written: a runtime diff has no versioned identity.
    ///
    /// Before executing anything, plans that rebuild a table with a new
    /// `NOT NULL` column that has no default are rejected when the table
    /// contains rows — otherwise the rebuild would fail mid-transaction.
    pub async fn apply_diff(
        &self,
        provider: &dyn IDatabaseProvider,
        diff: &MigrationDiff,
    ) -> EFResult<DiffApplyResult> {
        if diff.statements.is_empty() {
            return Ok(DiffApplyResult::default());
        }
        self.preflight_rebuilds(provider, diff).await?;

        match self.dialect {
            MigrationDialect::MySql => {
                for statement in &diff.statements {
                    provider.execute_migration_command(statement).await?;
                }
            }
            MigrationDialect::Sqlite => self.apply_sqlite(provider, diff).await?,
            MigrationDialect::Postgres => self.apply_transactionally(provider, diff).await?,
        }

        Ok(DiffApplyResult {
            executed_statements: diff.statements.len(),
            rebuilt_tables: diff
                .rebuilds
                .iter()
                .map(|rebuild| rebuild.table.clone())
                .collect(),
        })
    }

    /// Rejects plans that cannot preserve existing rows.
    async fn preflight_rebuilds(
        &self,
        provider: &dyn IDatabaseProvider,
        diff: &MigrationDiff,
    ) -> EFResult<()> {
        for rebuild in &diff.rebuilds {
            if rebuild.not_null_columns_without_default.is_empty() {
                continue;
            }
            let table = self.dialect.quote(&rebuild.table);
            let mut connection = provider.get_connection().await?;
            let rows = connection
                .query(&format!("SELECT COUNT(*) FROM {table}"), &[])
                .await?;
            let row_count = rows
                .first()
                .and_then(|row| row.first())
                .and_then(|value| i64::try_from(value.clone()).ok())
                .unwrap_or(0);
            if row_count > 0 {
                return Err(EFError::migration(format!(
                    "cannot rebuild table {}: new NOT NULL column(s) {} have no default but the table \
                     contains {row_count} row(s); add a default or backfill first",
                    rebuild.table,
                    rebuild.not_null_columns_without_default.join(", ")
                )));
            }
        }
        Ok(())
    }

    /// SQLite: one transaction with foreign keys disabled around it.
    async fn apply_sqlite(
        &self,
        provider: &dyn IDatabaseProvider,
        diff: &MigrationDiff,
    ) -> EFResult<()> {
        let mut connection = provider.get_connection().await?;
        let foreign_keys_were_on = read_pragma_bool(&mut *connection, "foreign_keys").await?;
        if foreign_keys_were_on {
            // Must happen outside the transaction: SQLite silently ignores
            // `PRAGMA foreign_keys` while a transaction is active.
            connection.execute("PRAGMA foreign_keys = OFF", &[]).await?;
        }

        let outcome = run_transaction(&mut *connection, diff, !diff.rebuilds.is_empty()).await;

        if foreign_keys_were_on {
            // Restore enforcement on the pooled connection before releasing it.
            let _ = connection.execute("PRAGMA foreign_keys = ON", &[]).await;
        }
        outcome
    }

    /// PostgreSQL: one transaction, no PRAGMA handling needed.
    async fn apply_transactionally(
        &self,
        provider: &dyn IDatabaseProvider,
        diff: &MigrationDiff,
    ) -> EFResult<()> {
        let mut connection = provider.get_connection().await?;
        run_transaction(&mut *connection, diff, false).await
    }
}

async fn run_transaction(
    connection: &mut dyn IAsyncConnection,
    diff: &MigrationDiff,
    check_foreign_keys: bool,
) -> EFResult<()> {
    connection.begin_transaction().await?;
    for statement in &diff.statements {
        if let Err(error) = connection.execute(statement, &[]).await {
            let _ = connection.rollback_transaction().await;
            return Err(error);
        }
    }

    if check_foreign_keys {
        // Defense in depth: a rebuild that changes constraints must not commit
        // a schema that leaves orphaned rows behind.
        let violations = connection.query("PRAGMA foreign_key_check", &[]).await?;
        if !violations.is_empty() {
            let _ = connection.rollback_transaction().await;
            return Err(EFError::migration(format!(
                "the runtime diff would leave {} foreign-key violation(s); every change was rolled back",
                violations.len()
            )));
        }
    }

    if let Err(error) = connection.commit_transaction().await {
        let _ = connection.rollback_transaction().await;
        return Err(error);
    }
    Ok(())
}

async fn read_pragma_bool(connection: &mut dyn IAsyncConnection, pragma: &str) -> EFResult<bool> {
    let rows = connection.query(&format!("PRAGMA {pragma}"), &[]).await?;
    let value = rows
        .first()
        .and_then(|row| row.first())
        .cloned()
        .unwrap_or(DbValue::Null);
    Ok(i64::try_from(value).unwrap_or(0) != 0)
}
