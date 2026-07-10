//! `ChangeExecutor` — INSERT and UPSERT execution.

use crate::entity::{IEntitySnapshot, IEntityType, IGetKeyValues};
use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};

/// Executes INSERT/UPDATE/DELETE for tracked entities within a transaction.
pub struct ChangeExecutor;

impl ChangeExecutor {
    /// Executes INSERT statements for all added entities.
    ///
    /// Rows are batched into multi-value `INSERT INTO ... VALUES (...), (...)`
    /// statements to minimize round trips. Batches are sized so that the total
    /// parameter count stays ≤ 900 (SQLite's variable limit is 999; we use a
    /// conservative ceiling that also fits MySQL/PG). Auto-increment columns
    /// are excluded from the column list (the DB assigns them).
    ///
    /// `on_key_backfill` is invoked once per inserted row with the
    /// database-generated auto-increment PK (when present). For PostgreSQL the
    /// PKs are read from the `RETURNING *` result set; for SQLite/MySQL a
    /// follow-up `last_insert_rowid()` / `LAST_INSERT_ID()` query retrieves
    /// the first/last ID and the remaining batch keys are computed by
    /// sequential increment.
    pub async fn execute_inserts<E, F>(
        conn: &mut dyn IAsyncConnection,
        provider: &dyn IDatabaseProvider,
        entities: &[(&E, &EntityTypeMeta)],
        mut on_key_backfill: F,
    ) -> EFResult<usize>
    where
        E: IEntityType + IEntitySnapshot + IGetKeyValues,
        F: FnMut(usize, i64),
    {
        if entities.is_empty() {
            return Ok(0);
        }
        let gen = provider.sql_generator();
        let meta = entities[0].1;
        let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
        if scalar_props.is_empty() {
            return Ok(0);
        }
        let insert_cols: Vec<&str> = scalar_props
            .iter()
            .filter(|p| !p.is_primary_key || (!p.is_auto_increment && !p.is_sequence))
            .map(|p| p.column_name.as_ref())
            .collect();
        if insert_cols.is_empty() {
            return Ok(0);
        }

        // Identify the auto-increment/sequence PK column (if any) for key backfill.
        let auto_inc_pk = scalar_props
            .iter()
            .find(|p| (p.is_auto_increment || p.is_sequence) && p.is_primary_key);

        // Conservative per-statement parameter ceiling (SQLite limit 999).
        const MAX_PARAMS: usize = 900;
        let batch_size = (MAX_PARAMS / insert_cols.len()).max(1);

        let mut inserted = 0usize;
        let mut start = 0usize;
        while start < entities.len() {
            let end = (start + batch_size).min(entities.len());
            let batch = &entities[start..end];
            let row_count = batch.len();

            let sql = gen.insert_batch(meta.table_name.as_ref(), &insert_cols, row_count);
            let mut params: Vec<DbValue> = Vec::with_capacity(row_count * insert_cols.len());
            for (entity, _) in batch {
                let snap = entity.snapshot();
                for p in &scalar_props {
                    if !p.is_primary_key || (!p.is_auto_increment && !p.is_sequence) {
                        params.push(
                            snap.get(p.field_name.as_ref())
                                .cloned()
                                .unwrap_or(DbValue::Null),
                        );
                    }
                }
            }

            if let Some(_pk_prop) = auto_inc_pk {
                if gen.supports_returning() {
                    // PostgreSQL: INSERT ... RETURNING * returns all generated
                    // rows. Extract the PK column from each returned row.
                    let rows = conn.query(&sql, &params).await?;
                    let affected = rows.len().min(row_count);
                    for (i, row) in rows.iter().enumerate().take(affected) {
                        if let Some(pk_val) = row.first() {
                            if let Ok(key) = pk_val.clone().try_into() {
                                on_key_backfill(start + i, key);
                            }
                        }
                    }
                    inserted += affected;
                } else if let Some(last_id_sql) = gen.last_insert_id_sql() {
                    // SQLite/MySQL: execute INSERT, then query for the
                    // generated ID and compute the batch key sequence.
                    conn.execute(&sql, &params).await?;
                    let id_rows = conn.query(last_id_sql, &[]).await?;
                    if let Some(id_row) = id_rows.first() {
                        if let Some(id_val) = id_row.first() {
                            let raw_id: i64 = id_val.clone().try_into().unwrap_or(0);
                            let first_id = if gen.last_insert_id_returns_first() {
                                raw_id
                            } else {
                                raw_id - row_count as i64 + 1
                            };
                            for i in 0..row_count {
                                on_key_backfill(start + i, first_id + i as i64);
                            }
                        }
                    }
                    inserted += row_count;
                } else {
                    // No RETURNING and no last_insert_id — cannot backfill.
                    let rows = conn.execute(&sql, &params).await?;
                    let affected = (rows as usize).min(row_count);
                    for i in 0..affected {
                        on_key_backfill(start + i, 0);
                    }
                    inserted += affected;
                }
            } else {
                // No auto-increment PK — entity already has its key.
                let rows = conn.execute(&sql, &params).await?;
                let affected = (rows as usize).min(row_count);
                for i in 0..affected {
                    on_key_backfill(start + i, 0);
                }
                inserted += affected;
            }

            start = end;
        }

        Ok(inserted)
    }

    /// Executes UPSERT statements (INSERT ... ON CONFLICT DO UPDATE) for
    /// entities marked via `DbSet::upsert`.
    ///
    /// Unlike `execute_inserts`, ALL mapped columns (including auto-increment
    /// PKs) are included in the INSERT column list so the caller-provided PK
    /// value participates in the conflict check. The conflict target is the PK
    /// column(s). The UPDATE SET clause covers all non-PK columns.
    /// No PK backfill is performed — upsert callers are expected to know the key.
    pub async fn execute_upserts<E>(
        conn: &mut dyn IAsyncConnection,
        provider: &dyn IDatabaseProvider,
        entities: &[(&E, &EntityTypeMeta)],
    ) -> EFResult<usize>
    where
        E: IEntityType + IEntitySnapshot + IGetKeyValues,
    {
        if entities.is_empty() {
            return Ok(0);
        }
        let gen = provider.sql_generator();
        let meta = entities[0].1;
        let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
        if scalar_props.is_empty() {
            return Ok(0);
        }
        // For upsert, include ALL mapped columns (including auto-increment PK)
        // so the caller's key value is part of the INSERT and can trigger the
        // ON CONFLICT path.
        let insert_cols: Vec<&str> = scalar_props
            .iter()
            .map(|p| p.column_name.as_ref())
            .collect();
        if insert_cols.is_empty() {
            return Ok(0);
        }

        // Conflict target: PK column names.
        let conflict_cols: Vec<&str> = scalar_props
            .iter()
            .filter(|p| p.is_primary_key)
            .map(|p| p.column_name.as_ref())
            .collect();
        if conflict_cols.is_empty() {
            return Err(crate::error::EFError::configuration(
                "Upsert requires at least one primary key column as conflict target.",
            ));
        }

        const MAX_PARAMS: usize = 900;
        let batch_size = (MAX_PARAMS / insert_cols.len()).max(1);

        let mut upserted = 0usize;
        let mut start = 0usize;
        while start < entities.len() {
            let end = (start + batch_size).min(entities.len());
            let batch = &entities[start..end];
            let row_count = batch.len();

            let sql = gen.upsert_batch(
                meta.table_name.as_ref(),
                &insert_cols,
                &conflict_cols,
                row_count,
            );
            let mut params: Vec<DbValue> = Vec::with_capacity(row_count * insert_cols.len());
            for (entity, _) in batch {
                let snap = entity.snapshot();
                for p in &scalar_props {
                    params.push(
                        snap.get(p.field_name.as_ref())
                            .cloned()
                            .unwrap_or(DbValue::Null),
                    );
                }
            }

            let affected = conn.execute(&sql, &params).await?;
            upserted += (affected as usize).min(row_count);

            start = end;
        }

        Ok(upserted)
    }
}
