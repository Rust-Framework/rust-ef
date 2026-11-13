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
//! | MySQL | DDL implicitly commits, so statements are executed individually through the shared statement executor ([`MigrationEngine::execute_statements`]). Partial application is possible (engine limitation, see EF-13), and a failure returns the same [`PartialMigrationFailure`](super::engine_ops::PartialMigrationFailure) report as the versioned `apply` path: executed / failed / not-executed statements. |

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
            // PostgreSQL: one transaction. MySQL: statement-by-statement with
            // the shared EF-13 partial-failure report. Both share the exact
            // execution plan used by the versioned migration workflow.
            MigrationDialect::MySql | MigrationDialect::Postgres => {
                self.execute_statements(provider, &diff.statements).await?;
            }
            MigrationDialect::Sqlite => self.apply_sqlite(provider, diff).await?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::live_diff::MigrationDiff;
    use crate::provider::ISqlGenerator;
    use std::sync::Mutex;

    /// MySQL provider double: records DDL statements and fails at a fixed call.
    struct FailingProvider {
        fail_at: usize,
        calls: Mutex<Vec<String>>,
    }

    impl FailingProvider {
        fn failing_at(fail_at: usize) -> Self {
            Self {
                fail_at,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl IDatabaseProvider for FailingProvider {
        fn sql_generator(&self) -> &'static dyn ISqlGenerator {
            unimplemented!("apply_diff's MySQL path does not use the SQL generator")
        }

        async fn get_connection(&self) -> EFResult<Box<dyn IAsyncConnection>> {
            unimplemented!("apply_diff's MySQL path does not need a connection")
        }

        async fn execute_migration_command(&self, sql: &str) -> EFResult<()> {
            let mut calls = self.calls.lock().expect("calls lock");
            calls.push(sql.to_string());
            if calls.len() == self.fail_at {
                return Err(EFError::query("simulated DDL failure"));
            }
            Ok(())
        }

        fn name(&self) -> &str {
            "failing"
        }

        fn migration_dialect(&self) -> MigrationDialect {
            MigrationDialect::MySql
        }
    }

    fn partial_failure(error: &EFError) -> &crate::migration::PartialMigrationFailure {
        std::error::Error::source(error)
            .and_then(|source| source.downcast_ref())
            .expect("error source must be the partial-migration report")
    }

    #[tokio::test]
    async fn mysql_apply_diff_reports_executed_and_pending_statements() {
        let provider = FailingProvider::failing_at(2);
        let engine = MigrationEngine::new(MigrationDialect::MySql);
        let diff = MigrationDiff {
            statements: vec![
                "CREATE TABLE diff_a (id INT)".to_string(),
                "CREATE TABLE diff_b (id INT)".to_string(),
                "CREATE TABLE diff_c (id INT)".to_string(),
            ],
            warnings: Vec::new(),
            rebuilds: Vec::new(),
        };

        let error = engine
            .apply_diff(&provider, &diff)
            .await
            .expect_err("the second statement fails");

        // The runtime diff path reuses the shared executor, so the EF-13
        // partial-failure report is available exactly like for `apply()`.
        let failure = partial_failure(&error);
        assert_eq!(
            failure.executed,
            vec!["CREATE TABLE diff_a (id INT)"],
            "only the committed statements are reported"
        );
        assert_eq!(failure.failed, "CREATE TABLE diff_b (id INT)");
        assert_eq!(
            failure.not_executed,
            vec!["CREATE TABLE diff_c (id INT)"],
            "statements after the failure are reported as not executed"
        );
        assert!(failure.cause.contains("simulated DDL failure"));
        assert_eq!(
            provider.calls(),
            vec![
                "CREATE TABLE diff_a (id INT)",
                "CREATE TABLE diff_b (id INT)"
            ],
            "execution stops at the failing statement"
        );
        assert!(
            error.message().contains("partially applied"),
            "the error message must state the non-atomic outcome: {}",
            error.message()
        );
    }

    #[tokio::test]
    async fn mysql_apply_diff_without_failure_returns_all_statements() {
        let provider = FailingProvider::failing_at(usize::MAX);
        let engine = MigrationEngine::new(MigrationDialect::MySql);
        let diff = MigrationDiff {
            statements: vec!["CREATE TABLE diff_a (id INT)".to_string()],
            warnings: Vec::new(),
            rebuilds: Vec::new(),
        };

        let result = engine.apply_diff(&provider, &diff).await.expect("apply");
        assert_eq!(result.executed_statements, 1);
        assert!(result.rebuilt_tables.is_empty());
        assert_eq!(provider.calls().len(), 1);
    }
}
