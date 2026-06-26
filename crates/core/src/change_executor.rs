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
    /// Returns the number of rows inserted.
    /// For auto-increment columns, the generated key values are written back
    /// via the `on_key_backfill` callback.
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
        let gen = provider.sql_generator();
        let mut inserted = 0;

        for (idx, (entity, meta)) in entities.iter().enumerate() {
            let snap = entity.snapshot();
            let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
            if scalar_props.is_empty() {
                continue;
            }

            let insert_cols: Vec<&str> = scalar_props
                .iter()
                .filter(|p| !p.is_auto_increment || !p.is_primary_key)
                .map(|p| p.column_name.as_ref())
                .collect();

            let params: Vec<DbValue> = scalar_props
                .iter()
                .filter(|p| !p.is_auto_increment || !p.is_primary_key)
                .map(|p| {
                    snap.get(p.field_name.as_ref())
                        .cloned()
                        .unwrap_or(DbValue::Null)
                })
                .collect();

            if insert_cols.is_empty() {
                continue;
            }

            let sql = gen.insert(meta.table_name.as_ref(), &insert_cols, true);
            let rows = conn.execute(&sql, &params).await?;

            if rows > 0 {
                on_key_backfill(idx, rows as i64);
                inserted += 1;
            }
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
                &*gen,
                &keys,
                &concurrency_tokens,
                *original,
                set_cols.len() + 1,
            )?;

            // Append the query filter (e.g. tenant_id = ?) to the WHERE clause.
            // Filter param placeholders are indexed after SET cols + existing WHERE params.
            if let Some(filter) = query_filter {
                let mut idx = set_cols.len() + where_params.len() + 1;
                let filter_sql = compile_bool_expr(filter, &*gen, &mut idx);
                where_params.extend(collect_bool_expr_values(filter));
                where_clause = format!("({}) AND ({})", where_clause, filter_sql);
            }

            let sql = gen.update(meta.table_name.as_ref(), &set_cols, &where_clause);

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
                return Err(EFError::ConcurrencyConflict(format!(
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
        let gen = provider.sql_generator();
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
                build_where_with_concurrency(&*gen, &keys, &concurrency_tokens, *original, 1)?;

            // Append the query filter to the WHERE clause.
            if let Some(filter) = query_filter {
                let mut idx = where_params.len() + 1;
                let filter_sql = compile_bool_expr(filter, &*gen, &mut idx);
                where_params.extend(collect_bool_expr_values(filter));
                where_clause = format!("({}) AND ({})", where_clause, filter_sql);
            }

            let sql = gen.delete(meta.table_name.as_ref(), &where_clause);
            let rows = conn.execute(&sql, &where_params).await?;
            if rows == 0 {
                return Err(EFError::ConcurrencyConflict(format!(
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
    let mut next_idx = start_param_idx + keys.len();

    for token in concurrency_tokens {
        where_parts.push(format!(
            "{} = {}",
            gen.quote_identifier(token.column_name.as_ref()),
            gen.parameter_placeholder(next_idx)
        ));
        next_idx += 1;

        let original_val = original
            .and_then(|o| o.get(token.field_name.as_ref()))
            .ok_or_else(|| {
                EFError::ChangeTracking(format!(
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
