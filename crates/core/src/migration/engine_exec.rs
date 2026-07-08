//! Async execution methods for `MigrationEngine`.
//!
//! Applies generated SQL to a database connection, tracks applied
//! migrations in `__ef_migrations_history`, and supports revert/rollback.
//! Split from the main engine module for readability — these methods
//! perform I/O against `IDatabaseProvider`.
//!
//! This is a third `impl MigrationEngine` block; the struct definition
//! lives in `engine.rs`.

use std::collections::{HashMap, HashSet};

use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::IDatabaseProvider;

use super::history::{create_migration_history_table_sql, seed_insert_sql, split_sql_statements};
use super::types::{Migration, MigrationHistoryEntry, MIGRATION_HISTORY_TABLE};

impl MigrationEngine {
    /// Ensures the migration history table exists.
    pub async fn ensure_history_table(
        &self,
        provider: &dyn IDatabaseProvider,
    ) -> EFResult<()> {
        let sql = create_migration_history_table_sql(self.dialect);
        provider.execute_migration_command(&sql).await
    }

    /// Applies a migration's Up SQL against a database provider.
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
        for statement in split_sql_statements(&sql) {
            if !statement.is_empty() {
                provider.execute_migration_command(&statement).await?;
            }
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
            return Err(crate::error::EFError::Migration(format!(
                "migration '{}' is not applied",
                migration.id
            )));
        }
        let sql = migration.down_sql.replace("{migration_id}", &migration.id);
        for statement in split_sql_statements(&sql) {
            if !statement.is_empty() {
                provider.execute_migration_command(&statement).await?;
            }
        }
        Ok(())
    }

    /// Returns migrations already recorded in the database history table.
    pub async fn get_applied_migrations(
        &self,
        provider: &dyn IDatabaseProvider,
    ) -> EFResult<Vec<MigrationHistoryEntry>> {
        self.ensure_history_table(provider).await?;
        let q = |s: &str| self.dialect.quote(s);
        let table = q(MIGRATION_HISTORY_TABLE);
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
        let applied: HashSet<String> = self
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
            crate::error::EFError::Migration(format!(
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
                    crate::error::EFError::Migration(format!(
                        "target migration '{t}' is not applied"
                    ))
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
                crate::error::EFError::Migration(format!(
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
                crate::error::EFError::Migration(format!(
                    "from migration '{f}' not found in local set"
                ))
            })? as i64,
            None => -1,
        };
        let to_idx: i64 = match to {
            Some(t) => migrations.iter().position(|m| m.id == t).ok_or_else(|| {
                crate::error::EFError::Migration(format!(
                    "to migration '{t}' not found in local set"
                ))
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
        for statement in split_sql_statements(&sql) {
            if !statement.is_empty() {
                provider.execute_migration_command(&statement).await?;
            }
        }
        Ok(())
    }

    /// Drops all tables for the given entity types.
    /// Corresponds to EF Core `Database.EnsureDeleted()`.
    pub async fn ensure_deleted(
        &self,
        provider: &dyn IDatabaseProvider,
        entity_types: &[EntityTypeMeta],
    ) -> EFResult<()> {
        let q = |s: &str| self.dialect.quote(s);
        for meta in entity_types {
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
        rows: &[HashMap<String, crate::provider::DbValue>],
    ) -> EFResult<()> {
        if rows.is_empty() {
            return Ok(());
        }

        let gen = provider.sql_generator();
        let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
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
