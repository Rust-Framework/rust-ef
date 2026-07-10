//! Standalone SQL generation helpers (for use by simplified callers).

use crate::entity_snapshot::EntitySnapshot;
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, IDatabaseProvider};

pub fn generate_insert_sql(
    provider: &dyn IDatabaseProvider,
    meta: &EntityTypeMeta,
    _property_values: &EntitySnapshot,
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
    property_values: &EntitySnapshot,
    primary_key_values: &EntitySnapshot,
) -> String {
    let gen = provider.sql_generator();
    let set_columns: Vec<&str> = property_values
        .iter()
        .map(|(k, _)| k)
        .filter(|k| primary_key_values.get(k).is_none())
        .collect();
    if set_columns.is_empty() || primary_key_values.is_empty() {
        return String::new();
    }
    let where_parts: Vec<String> = primary_key_values
        .iter()
        .enumerate()
        .map(|(i, (k, _))| {
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
    primary_key_values: &EntitySnapshot,
) -> String {
    let gen = provider.sql_generator();
    if primary_key_values.is_empty() {
        return String::new();
    }
    let where_parts: Vec<String> = primary_key_values
        .iter()
        .enumerate()
        .map(|(i, (k, _))| {
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
    property_values: &EntitySnapshot,
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
    property_values: &EntitySnapshot,
    primary_key_values: &EntitySnapshot,
    set_keys: &[String],
) -> Vec<DbValue> {
    let mut params: Vec<DbValue> = set_keys
        .iter()
        .filter(|k| primary_key_values.get(k.as_str()).is_none())
        .map(|k| {
            property_values
                .get(k.as_str())
                .cloned()
                .unwrap_or(DbValue::Null)
        })
        .collect();
    for (_, v) in primary_key_values.iter() {
        params.push(v.clone());
    }
    params
}

pub fn collect_delete_params(primary_key_values: &EntitySnapshot) -> Vec<DbValue> {
    primary_key_values.iter().map(|(_, v)| v.clone()).collect()
}
