//! Change executor — generates and executes SQL for entity state changes.
//!
//! The `ChangeExecutor` takes a collection of tracked entities grouped by
//! state (Added/Modified/Deleted), generates the appropriate parameterized
//! DML, and executes it against the database via the provider.

use crate::entity::{IEntitySnapshot, IEntityType, IGetKeyValues};
use crate::error::{EFError, EFResult};
use crate::metadata::{EntityTypeMeta, PropertyMeta};
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};
use crate::query::{collect_bool_expr_values, compile_bool_expr, BoolExpr};
use std::collections::HashMap;

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
    /// `on_key_backfill` is invoked once per inserted row. In batch mode the
    /// per-row generated key is not available, so `0` is passed as the key
    /// (the callback is currently a no-op in the framework).
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
            .filter(|p| !p.is_auto_increment || !p.is_primary_key)
            .map(|p| p.column_name.as_ref())
            .collect();
        if insert_cols.is_empty() {
            return Ok(0);
        }

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
                    if !p.is_auto_increment || !p.is_primary_key {
                        params.push(
                            snap.get(p.field_name.as_ref())
                                .cloned()
                                .unwrap_or(DbValue::Null),
                        );
                    }
                }
            }

            let rows = conn.execute(&sql, &params).await?;
            let affected = (rows as usize).min(row_count);
            for i in 0..affected {
                on_key_backfill(start + i, 0);
            }
            inserted += affected;
            start = end;
        }

        Ok(inserted)
    }

    /// Executes UPDATE statements for all modified entities.
    /// Uses original snapshots for optimistic concurrency tokens in the WHERE clause.
    ///
    /// When `query_filter` is `Some`, the filter (e.g. a tenant-id predicate)
    /// is AND-ed into the WHERE clause so updates cannot cross the filter
    /// boundary (multi-tenant / soft-delete isolation).
    #[allow(clippy::type_complexity)]
    pub async fn execute_updates<E>(
        conn: &mut dyn IAsyncConnection,
        provider: &dyn IDatabaseProvider,
        entities: &[(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)],
        query_filter: Option<&BoolExpr>,
    ) -> EFResult<usize>
    where
        E: IEntityType + IEntitySnapshot + IGetKeyValues,
    {
        let gen = provider.sql_generator();
        let mut updated = 0;
        // Cache of `(table, set_cols, where_clause) -> UPDATE SQL template`.
        // For N rows of the same entity type the SQL string is identical (only
        // the bound parameter values differ), so this avoids N-1 redundant
        // `format!` calls. Scoped to this call (DbContext lifetime).
        let mut sql_cache: HashMap<(String, Vec<String>, String), String> = HashMap::new();

        for (entity, meta, original) in entities {
            let snap = entity.snapshot();
            let keys = entity.key_values();
            let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();

            let set_cols: Vec<&str> = scalar_props
                .iter()
                .filter(|p| !p.is_primary_key)
                .map(|p| p.column_name.as_ref())
                .collect();

            if set_cols.is_empty() || keys.is_empty() {
                continue;
            }

            let concurrency_tokens: Vec<&PropertyMeta> = scalar_props
                .iter()
                .copied()
                .filter(|p| p.is_concurrency_token)
                .collect();

            let (mut where_clause, mut where_params) = build_where_with_concurrency(
                gen,
                &keys,
                &concurrency_tokens,
                *original,
                set_cols.len() + 1,
            )?;

            // Append the query filter (e.g. tenant_id = ?) to the WHERE clause.
            // Filter param placeholders are indexed after SET cols + existing WHERE params.
            if let Some(filter) = query_filter {
                let mut idx = set_cols.len() + where_params.len() + 1;
                let filter_sql = compile_bool_expr(filter, gen, &mut idx);
                where_params.extend(collect_bool_expr_values(filter));
                where_clause = format!("({}) AND ({})", where_clause, filter_sql);
            }

            let sql = sql_cache
                .entry((
                    meta.table_name.to_string(),
                    set_cols.iter().map(|s| (*s).to_string()).collect(),
                    where_clause.clone(),
                ))
                .or_insert_with(|| gen.update(meta.table_name.as_ref(), &set_cols, &where_clause))
                .clone();

            let mut params: Vec<DbValue> = set_cols
                .iter()
                .map(|col| {
                    let prop = scalar_props.iter().find(|p| p.column_name.as_ref() == *col);
                    match prop {
                        Some(p) => snap
                            .get(p.field_name.as_ref())
                            .cloned()
                            .unwrap_or(DbValue::Null),
                        None => DbValue::Null,
                    }
                })
                .collect();
            params.extend(where_params);

            let rows = conn.execute(&sql, &params).await?;
            if rows == 0 {
                return Err(EFError::concurrency_conflict(format!(
                    "update affected 0 rows on {} (row may have been modified or deleted)",
                    meta.table_name
                )));
            }
            updated += 1;
        }

        Ok(updated)
    }

    /// Executes DELETE statements for all deleted entities.
    ///
    /// When no concurrency tokens are present and the entity has a single-column
    /// primary key, rows are batched into `DELETE ... WHERE pk IN (?, ?, ...)`
    /// statements (≤900 params per batch) to minimize round trips. Otherwise
    /// (concurrency tokens, composite PK), falls back to per-row DELETE so
    /// optimistic-concurrency checks run on each row.
    ///
    /// When `query_filter` is `Some`, the filter is AND-ed into the WHERE
    /// clause so deletes cannot cross the filter boundary.
    #[allow(clippy::type_complexity)]
    pub async fn execute_deletes<E>(
        conn: &mut dyn IAsyncConnection,
        provider: &dyn IDatabaseProvider,
        entities: &[(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)],
        query_filter: Option<&BoolExpr>,
    ) -> EFResult<usize>
    where
        E: IEntityType + IGetKeyValues,
    {
        if entities.is_empty() {
            return Ok(0);
        }
        let gen = provider.sql_generator();
        let meta = entities[0].1;
        let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
        let has_concurrency_tokens = scalar_props.iter().any(|p| p.is_concurrency_token);
        let pk_props: Vec<_> = scalar_props.iter().filter(|p| p.is_primary_key).collect();

        // Fall back to per-row DELETE when optimistic concurrency tokens are
        // present (each row needs its own WHERE to check the token) or when
        // the PK is composite (IN clause only works for a single column).
        if has_concurrency_tokens || pk_props.len() != 1 {
            return Self::execute_deletes_per_row(conn, gen, entities, query_filter).await;
        }

        let pk_col = pk_props[0].column_name.as_ref();
        let pk_field = pk_props[0].field_name.as_ref();

        // Collect PK values; entities missing the PK value are skipped.
        let pk_values: Vec<DbValue> = entities
            .iter()
            .filter_map(|(e, _, _)| e.key_values().get(pk_field).cloned())
            .collect();
        if pk_values.is_empty() {
            return Ok(0);
        }

        // Filter params are constant across batches; their SQL is recomputed
        // per batch because Postgres numbered placeholders depend on batch size.
        let filter_params: Vec<DbValue> = match query_filter {
            Some(filter) => collect_bool_expr_values(filter),
            None => Vec::new(),
        };
        const MAX_PARAMS: usize = 900;
        let batch_size = MAX_PARAMS.saturating_sub(filter_params.len()).max(1);

        let mut deleted = 0usize;
        let mut start = 0usize;
        while start < pk_values.len() {
            let end = (start + batch_size).min(pk_values.len());
            let batch = &pk_values[start..end];
            let pk_count = batch.len();

            let pk_placeholders: Vec<String> = (1..=pk_count)
                .map(|i| gen.parameter_placeholder(i))
                .collect();
            let mut where_clause = format!(
                "{} IN ({})",
                gen.quote_identifier(pk_col),
                pk_placeholders.join(", "),
            );
            let mut params: Vec<DbValue> = batch.to_vec();
            if let Some(filter) = query_filter {
                // Filter placeholders are numbered after the PK IN-list.
                let mut idx = pk_count + 1;
                let filter_sql = compile_bool_expr(filter, gen, &mut idx);
                params.extend(filter_params.iter().cloned());
                where_clause = format!("({}) AND ({})", where_clause, filter_sql);
            }

            let sql = gen.delete(meta.table_name.as_ref(), &where_clause);
            let rows = conn.execute(&sql, &params).await?;
            if rows == 0 && pk_count > 0 {
                return Err(EFError::concurrency_conflict(format!(
                    "batch delete affected 0 rows on {} (rows may have been modified or deleted)",
                    meta.table_name
                )));
            }
            deleted += (rows as usize).min(pk_count);
            start = end;
        }

        Ok(deleted)
    }

    /// Per-row DELETE fallback used when concurrency tokens or composite PKs
    /// prevent batching. Each row gets its own `DELETE ... WHERE pk = ? AND ...`.
    #[allow(clippy::type_complexity)]
    async fn execute_deletes_per_row<E>(
        conn: &mut dyn IAsyncConnection,
        gen: &'static dyn crate::provider::ISqlGenerator,
        entities: &[(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)],
        query_filter: Option<&BoolExpr>,
    ) -> EFResult<usize>
    where
        E: IEntityType + IGetKeyValues,
    {
        let mut deleted = 0;
        for (entity, meta, original) in entities {
            let keys = entity.key_values();
            if keys.is_empty() {
                continue;
            }
            let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
            let concurrency_tokens: Vec<&PropertyMeta> = scalar_props
                .iter()
                .copied()
                .filter(|p| p.is_concurrency_token)
                .collect();

            let (mut where_clause, mut where_params) =
                build_where_with_concurrency(gen, &keys, &concurrency_tokens, *original, 1)?;

            if let Some(filter) = query_filter {
                let mut idx = where_params.len() + 1;
                let filter_sql = compile_bool_expr(filter, gen, &mut idx);
                where_params.extend(collect_bool_expr_values(filter));
                where_clause = format!("({}) AND ({})", where_clause, filter_sql);
            }

            let sql = gen.delete(meta.table_name.as_ref(), &where_clause);
            let rows = conn.execute(&sql, &where_params).await?;
            if rows == 0 {
                return Err(EFError::concurrency_conflict(format!(
                    "delete affected 0 rows on {} (row may have been modified or deleted)",
                    meta.table_name
                )));
            }
            deleted += 1;
        }
        Ok(deleted)
    }
}

fn build_where_with_concurrency(
    gen: &dyn crate::provider::ISqlGenerator,
    keys: &HashMap<String, DbValue>,
    concurrency_tokens: &[&PropertyMeta],
    original: Option<&HashMap<String, DbValue>>,
    start_param_idx: usize,
) -> EFResult<(String, Vec<DbValue>)> {
    let mut where_parts: Vec<String> = keys
        .keys()
        .enumerate()
        .map(|(i, k)| {
            format!(
                "{} = {}",
                gen.quote_identifier(k),
                gen.parameter_placeholder(start_param_idx + i)
            )
        })
        .collect();

    let mut params: Vec<DbValue> = keys.values().cloned().collect();

    for (next_idx, token) in (start_param_idx + keys.len()..).zip(concurrency_tokens.iter()) {
        where_parts.push(format!(
            "{} = {}",
            gen.quote_identifier(token.column_name.as_ref()),
            gen.parameter_placeholder(next_idx)
        ));

        let original_val = original
            .and_then(|o| o.get(token.field_name.as_ref()))
            .ok_or_else(|| {
                EFError::change_tracking(format!(
                    "missing original concurrency token for '{}'",
                    token.field_name
                ))
            })?;
        params.push(original_val.clone());
    }

    Ok((where_parts.join(" AND "), params))
}

// ---------------------------------------------------------------------------
// Standalone SQL generation helpers (for use by simplified callers)
// ---------------------------------------------------------------------------

pub fn generate_insert_sql(
    provider: &dyn IDatabaseProvider,
    meta: &EntityTypeMeta,
    _property_values: &HashMap<String, DbValue>,
) -> String {
    let gen = provider.sql_generator();
    let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
    let columns: Vec<&str> = scalar_props
        .iter()
        .map(|p| p.column_name.as_ref())
        .collect();
    if columns.is_empty() {
        return String::new();
    }
    gen.insert(meta.table_name.as_ref(), &columns, true)
}

pub fn generate_update_sql(
    provider: &dyn IDatabaseProvider,
    meta: &EntityTypeMeta,
    property_values: &HashMap<String, DbValue>,
    primary_key_values: &HashMap<String, DbValue>,
) -> String {
    let gen = provider.sql_generator();
    let set_columns: Vec<&str> = property_values
        .keys()
        .filter(|k| !primary_key_values.contains_key(*k))
        .map(|k| k.as_str())
        .collect();
    if set_columns.is_empty() || primary_key_values.is_empty() {
        return String::new();
    }
    let where_parts: Vec<String> = primary_key_values
        .keys()
        .enumerate()
        .map(|(i, k)| {
            format!(
                "{} = {}",
                gen.quote_identifier(k),
                gen.parameter_placeholder(i + 1)
            )
        })
        .collect();
    gen.update(
        meta.table_name.as_ref(),
        &set_columns,
        &where_parts.join(" AND "),
    )
}

pub fn generate_delete_sql(
    provider: &dyn IDatabaseProvider,
    meta: &EntityTypeMeta,
    primary_key_values: &HashMap<String, DbValue>,
) -> String {
    let gen = provider.sql_generator();
    if primary_key_values.is_empty() {
        return String::new();
    }
    let where_parts: Vec<String> = primary_key_values
        .keys()
        .enumerate()
        .map(|(i, k)| {
            format!(
                "{} = {}",
                gen.quote_identifier(k),
                gen.parameter_placeholder(i + 1)
            )
        })
        .collect();
    gen.delete(meta.table_name.as_ref(), &where_parts.join(" AND "))
}

pub fn collect_insert_params(
    meta: &EntityTypeMeta,
    property_values: &HashMap<String, DbValue>,
) -> Vec<DbValue> {
    meta.mapped_scalar_properties()
        .map(|p| {
            property_values
                .get(p.field_name.as_ref())
                .cloned()
                .unwrap_or(DbValue::Null)
        })
        .collect()
}

pub fn collect_update_params(
    property_values: &HashMap<String, DbValue>,
    primary_key_values: &HashMap<String, DbValue>,
    set_keys: &[String],
) -> Vec<DbValue> {
    let mut params: Vec<DbValue> = set_keys
        .iter()
        .filter(|k| !primary_key_values.contains_key(*k))
        .map(|k| property_values.get(k).cloned().unwrap_or(DbValue::Null))
        .collect();
    for v in primary_key_values.values() {
        params.push(v.clone());
    }
    params
}

pub fn collect_delete_params(primary_key_values: &HashMap<String, DbValue>) -> Vec<DbValue> {
    primary_key_values.values().cloned().collect()
}
