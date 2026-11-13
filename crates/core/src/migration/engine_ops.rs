//! `MigrationEngine` database operations — apply/revert migrations, ensure schema, seed data.

use super::dialect::MigrationDialect;
use super::engine::MigrationEngine;
use super::history::{
    create_migration_history_table_sql, seed_insert_sql, split_sql_statements,
    MigrationHistoryEntry,
};
use super::types::Migration;
use crate::entity_snapshot::EntitySnapshot;
use crate::error::{EFError, EFResult};
use crate::metadata::EntityTypeMeta;
use crate::provider::IDatabaseProvider;

/// Seed rows declared for one entity type, as produced by
/// [`crate::model_builder::EntityTypeBuilder::has_data`].
#[derive(Debug, Clone, Copy)]
pub struct SeedData<'a> {
    /// Entity metadata the rows belong to.
    pub meta: &'a EntityTypeMeta,
    /// Rows in insertion order.
    pub rows: &'a [EntitySnapshot],
}

impl MigrationEngine {
    /// Ensures the migration history table exists.
    pub async fn ensure_history_table(&self, provider: &dyn IDatabaseProvider) -> EFResult<()> {
        let sql = create_migration_history_table_sql(self.dialect);
        provider.execute_migration_command(&sql).await
    }

    /// Applies a migration's Up SQL against a database provider.
    ///
    /// PostgreSQL/SQLite execute the whole script (including the history row)
    /// in one transaction, so a failed statement rolls the migration back
    /// atomically. MySQL DDL implicitly commits, so its statements are executed
    /// individually (partial application is possible; the migration stays
    /// unrecorded and must be repaired manually).
    pub async fn apply(
        &self,
        provider: &dyn IDatabaseProvider,
        migration: &Migration,
    ) -> EFResult<()> {
        if self.is_applied(provider, &migration.id).await? {
            return Ok(());
        }
        self.ensure_history_table(provider).await?;
        let sql = migration.up_sql.replace("{migration_id}", &migration.id);
        self.execute_script(provider, &sql).await
    }

    /// Creates the full initial schema (tables, indexes and foreign keys) on an
    /// **empty** database without touching `__ef_migrations_history`.
    ///
    /// This is the history-free bootstrap path (EF Core's `EnsureCreated`
    /// equivalent for the migration engine): unlike [`Self::apply`] it never
    /// creates the history table and the generated script contains no history
    /// `INSERT`. A code-first installer therefore does not have to filter the
    /// generated script or drop the history table afterwards.
    ///
    /// Semantics:
    /// - the target database must be empty: `CREATE TABLE` is strict (not
    ///   `IF NOT EXISTS`), so a non-empty schema fails loudly instead of
    ///   silently mixing an old schema with the model;
    /// - views are not created (same as [`Self::ensure_created`]);
    /// - MySQL DDL is non-transactional (see [`Self::apply`]); a failure leaves
    ///   partially created objects behind and reports the executed statements.
    pub async fn bootstrap(
        &self,
        provider: &dyn IDatabaseProvider,
        entity_types: &[EntityTypeMeta],
    ) -> EFResult<()> {
        if entity_types.is_empty() {
            return Err(EFError::model_validation(
                "bootstrap requires at least one entity type",
            ));
        }
        let snapshot = self.create_snapshot("__bootstrap__", entity_types);
        let changes = self.initial_create_with_fks(&snapshot);
        let sql = self.generate_bootstrap_sql(&changes);
        self.execute_script(provider, &sql).await
    }

    /// Bootstraps the schema and inserts rows declared with
    /// [`crate::model_builder::EntityTypeBuilder::has_data`] — the
    /// `bootstrap → seed` install path (EF-11).
    ///
    /// `seeds` is applied in slice order, so list principals before dependents
    /// when the schema enforces foreign keys (an empty database has no rows to
    /// violate them unless a seed references a later one). Views and empty
    /// row sets are skipped.
    pub async fn bootstrap_with_seed(
        &self,
        provider: &dyn IDatabaseProvider,
        entity_types: &[EntityTypeMeta],
        seeds: &[SeedData<'_>],
    ) -> EFResult<()> {
        self.bootstrap(provider, entity_types).await?;
        for seed in seeds {
            if seed.meta.is_view || seed.rows.is_empty() {
                continue;
            }
            self.apply_seed_data(provider, seed.meta, seed.rows)
                .await?;
        }
        Ok(())
    }

    /// Executes a migration script with the best available atomicity.
    async fn execute_script(&self, provider: &dyn IDatabaseProvider, sql: &str) -> EFResult<()> {
        let statements = split_sql_statements(sql, self.dialect);
        if statements.is_empty() {
            return Ok(());
        }

        // MySQL DDL forces an implicit commit per statement; a transaction
        // cannot make the script atomic. Keep the provider's DDL path (the
        // prepared-statement protocol rejects DDL).
        if self.dialect == MigrationDialect::MySql {
            for statement in statements {
                provider.execute_migration_command(&statement).await?;
            }
            return Ok(());
        }

        let mut conn = provider.get_connection().await?;
        conn.begin_transaction().await?;
        for statement in &statements {
            if let Err(err) = conn.execute(statement, &[]).await {
                let _ = conn.rollback_transaction().await;
                return Err(err);
            }
        }
        if let Err(err) = conn.commit_transaction().await {
            let _ = conn.rollback_transaction().await;
            return Err(err);
        }
        Ok(())
    }

    /// Reverts a migration using its Down SQL.
    pub async fn revert(
        &self,
        provider: &dyn IDatabaseProvider,
        migration: &Migration,
    ) -> EFResult<()> {
        if !self.is_applied(provider, &migration.id).await? {
            return Err(EFError::migration(format!(
                "migration '{}' is not applied",
                migration.id
            )));
        }
        let sql = migration.down_sql.replace("{migration_id}", &migration.id);
        self.execute_script(provider, &sql).await
    }

    /// Returns migrations already recorded in the database history table.
    pub async fn get_applied_migrations(
        &self,
        provider: &dyn IDatabaseProvider,
    ) -> EFResult<Vec<MigrationHistoryEntry>> {
        self.ensure_history_table(provider).await?;
        let q = |s: &str| self.dialect.quote(s);
        let table = q(super::history::MIGRATION_HISTORY_TABLE);
        let sql = format!(
            "SELECT {}, {} FROM {} ORDER BY {}",
            q("migration_id"),
            q("product_version"),
            table,
            q("migration_id")
        );
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &[]).await?;
        Ok(rows
            .into_iter()
            .map(|row| MigrationHistoryEntry {
                migration_id: row
                    .first()
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_default(),
                product_version: row
                    .get(1)
                    .and_then(|v| String::try_from(v.clone()).ok())
                    .unwrap_or_default(),
            })
            .collect())
    }

    /// Returns whether a migration id is already recorded in history.
    pub async fn is_applied(
        &self,
        provider: &dyn IDatabaseProvider,
        migration_id: &str,
    ) -> EFResult<bool> {
        let applied = self.get_applied_migrations(provider).await?;
        Ok(applied.iter().any(|e| e.migration_id == migration_id))
    }

    /// Applies all pending migrations in order, skipping already-applied ids.
    pub async fn apply_pending(
        &self,
        provider: &dyn IDatabaseProvider,
        migrations: &[Migration],
    ) -> EFResult<usize> {
        let applied: std::collections::HashSet<String> = self
            .get_applied_migrations(provider)
            .await?
            .into_iter()
            .map(|e| e.migration_id)
            .collect();
        let mut count = 0usize;
        for migration in migrations {
            if applied.contains(&migration.id) {
                continue;
            }
            self.apply(provider, migration).await?;
            count += 1;
        }
        Ok(count)
    }

    /// Reverts the most recently applied migration from the given list.
    pub async fn revert_last(
        &self,
        provider: &dyn IDatabaseProvider,
        migrations: &[Migration],
    ) -> EFResult<Option<String>> {
        let applied = self.get_applied_migrations(provider).await?;
        let last = applied.last().map(|e| e.migration_id.clone());
        let Some(last_id) = last else {
            return Ok(None);
        };
        let migration = migrations.iter().find(|m| m.id == last_id).ok_or_else(|| {
            EFError::migration(format!(
                "applied migration '{}' not found in local migration set",
                last_id
            ))
        })?;
        self.revert(provider, migration).await?;
        Ok(Some(last_id))
    }

    /// Reverts all migrations applied strictly after `target` (exclusive),
    /// leaving the database at `target`'s state. Returns the reverted ids in
    /// the order they were reverted (most-recent first).
    ///
    /// If `target` is `None`, reverts ALL applied migrations.
    /// Returns an error if `target` is not currently applied.
    pub async fn revert_to_target(
        &self,
        provider: &dyn IDatabaseProvider,
        migrations: &[Migration],
        target: Option<&str>,
    ) -> EFResult<Vec<String>> {
        let applied = self.get_applied_migrations(provider).await?;
        let applied_ids: Vec<&str> = applied.iter().map(|e| e.migration_id.as_str()).collect();

        let to_revert: Vec<String> = match target {
            Some(t) => {
                let idx = applied_ids.iter().position(|id| *id == t).ok_or_else(|| {
                    EFError::migration(format!("target migration '{t}' is not applied"))
                })?;
                applied_ids[idx + 1..]
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            }
            None => applied_ids.iter().map(|s| s.to_string()).collect(),
        };

        let mut reverted = Vec::with_capacity(to_revert.len());
        for id in to_revert.iter().rev() {
            let migration = migrations.iter().find(|m| m.id == *id).ok_or_else(|| {
                EFError::migration(format!(
                    "applied migration '{id}' not found in local migration set"
                ))
            })?;
            self.revert(provider, migration).await?;
            reverted.push(id.clone());
        }
        Ok(reverted)
    }

    /// Generates a combined SQL script transitioning the database from the
    /// `from` migration state to the `to` migration state.
    ///
    /// Semantics (mirroring `dotnet ef migrations script`):
    /// - `from` is exclusive: the DB is assumed to already be at `from`.
    /// - `to` is inclusive: the script brings the DB up to and including `to`.
    /// - `from = None`: start from an empty database (before any migration).
    /// - `to = None`: end at the latest migration.
    /// - When `from` precedes `to`: emits Up SQL for migrations in `(from, to]`.
    /// - When `from` follows `to`: emits Down SQL for migrations in `(to, from]`.
    /// - When `from == to`: emits a no-op comment.
    pub fn generate_script(
        migrations: &[Migration],
        from: Option<&str>,
        to: Option<&str>,
    ) -> EFResult<String> {
        // Resolve from/to to indices. from=None means "before first migration" (-1).
        // to=None means "after last migration" (migrations.len() - 1).
        let from_idx: i64 = match from {
            Some(f) => migrations.iter().position(|m| m.id == f).ok_or_else(|| {
                EFError::migration(format!("from migration '{f}' not found in local set"))
            })? as i64,
            None => -1,
        };
        let to_idx: i64 = match to {
            Some(t) => migrations.iter().position(|m| m.id == t).ok_or_else(|| {
                EFError::migration(format!("to migration '{t}' not found in local set"))
            })? as i64,
            None => migrations.len() as i64 - 1,
        };

        let mut sql = String::new();
        if from_idx < to_idx {
            // Forward: up scripts for migrations in (from_idx, to_idx].
            sql.push_str("-- Migration script (forward)\n\n");
            let start = (from_idx + 1) as usize;
            let end = (to_idx + 1) as usize; // exclusive
            for m in &migrations[start..end] {
                sql.push_str(&format!("-- Up: {}\n", m.id));
                sql.push_str(&m.up_sql.replace("{migration_id}", &m.id));
                sql.push('\n');
            }
        } else if from_idx > to_idx {
            // Reverse: down scripts for migrations in (to_idx, from_idx].
            sql.push_str("-- Migration script (reverse)\n\n");
            let start = to_idx as usize + 1; // inclusive
            let end = from_idx as usize + 1; // exclusive
            for m in migrations[start..end].iter().rev() {
                sql.push_str(&format!("-- Down: {}\n", m.id));
                sql.push_str(&m.down_sql.replace("{migration_id}", &m.id));
                sql.push('\n');
            }
        } else {
            sql.push_str("-- Nothing to do (from == to)\n");
        }
        Ok(sql)
    }

    /// Generates and applies the initial schema for the given entity types.
    /// Corresponds to EF Core `Database.EnsureCreated()`.
    /// Does not use the migrations history table (idempotent DDL only).
    pub async fn ensure_created(
        &self,
        provider: &dyn IDatabaseProvider,
        entity_types: &[EntityTypeMeta],
    ) -> EFResult<()> {
        let snapshot = self.create_snapshot("__ensure_created__", entity_types);
        let changes = self.initial_create(&snapshot);
        let sql = self.generate_ddl_sql(&changes);
        self.execute_script(provider, &sql).await
    }

    /// Drops all tables for the given entity types.
    /// Corresponds to EF Core `Database.EnsureDeleted()`.
    pub async fn ensure_deleted(
        &self,
        provider: &dyn IDatabaseProvider,
        entity_types: &[EntityTypeMeta],
    ) -> EFResult<()> {
        let q = |s: &str| self.dialect.quote(s);
        for meta in entity_types.iter().filter(|m| !m.is_view) {
            let sql = format!("DROP TABLE IF EXISTS {};", q(meta.table_name.as_ref()));
            provider.execute_migration_command(&sql).await?;
        }
        Ok(())
    }

    /// Inserts seed data configured via `ModelBuilder::has_data`.
    pub async fn apply_seed_data(
        &self,
        provider: &dyn IDatabaseProvider,
        meta: &EntityTypeMeta,
        rows: &[EntitySnapshot],
    ) -> EFResult<()> {
        if rows.is_empty() {
            return Ok(());
        }

        let gen = provider.sql_generator();
        let scalar_props: Vec<_> = meta.writable_scalar_properties().collect();
        if scalar_props.is_empty() {
            return Ok(());
        }

        let col_names: Vec<&str> = scalar_props
            .iter()
            .map(|p| p.column_name.as_ref())
            .collect();

        let mut conn = provider.get_connection().await?;
        for row in rows {
            let params: Vec<crate::provider::DbValue> = scalar_props
                .iter()
                .map(|p| {
                    row.get(p.field_name.as_ref())
                        .cloned()
                        .unwrap_or(crate::provider::DbValue::Null)
                })
                .collect();

            let sql = seed_insert_sql(self.dialect, meta.table_name.as_ref(), &col_names, gen);
            conn.execute(&sql, &params).await?;
        }
        Ok(())
    }
}
