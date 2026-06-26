//! Eager-loading of navigation properties via secondary queries.

use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use crate::error::EFResult;
use crate::metadata::{EntityTypeMeta, NavigationKind, NavigationMeta};
use crate::provider::{DbValue, IDatabaseProvider, ISqlGenerator};
use crate::query::{compile_bool_expr, CompiledFilter, IncludePath};
use std::any::TypeId;
use std::collections::{HashMap, HashSet};

/// Appends a query filter (e.g. tenant_id = ?) to a navigation SQL statement.
///
/// Uses the pre-collected `params` from `CompiledFilter` (collected once at
/// registration time) instead of traversing the `BoolExpr` tree per query.
/// The SQL fragment is still compiled per query because placeholder numbering
/// is dialect-specific and depends on the current parameter count.
fn apply_filter_to_sql(
    sql: &mut String,
    params: &mut Vec<DbValue>,
    related_table: &str,
    filter_map: Option<&HashMap<String, CompiledFilter>>,
    gen: &dyn ISqlGenerator,
) {
    if let Some(filter) = filter_map.and_then(|m| m.get(related_table)) {
        let mut idx = params.len() + 1;
        let filter_sql = compile_bool_expr(&filter.expr, gen, &mut idx);
        params.extend(filter.params.iter().cloned());
        *sql = format!("{} AND ({})", sql, filter_sql);
    }
}

/// Builds include paths from navigation field names using entity metadata.
pub fn include_paths_for<T: IEntityType, S: AsRef<str>>(
    navigation_names: &[S],
) -> Vec<IncludePath> {
    let meta = T::entity_meta();
    navigation_names
        .iter()
        .filter_map(|name| include_path_from_meta(&meta, name.as_ref()))
        .collect()
}

fn include_path_from_meta(meta: &EntityTypeMeta, navigation: &str) -> Option<IncludePath> {
    let nav_meta = meta.find_navigation(navigation)?;
    Some(IncludePath {
        navigation: navigation.to_string(),
        nested: Vec::new(),
        related_table: nav_meta.related_table.as_ref().map(|s| s.to_string()),
        foreign_key_column: nav_meta.fk_column.as_ref().map(|s| s.to_string()),
        referenced_key_column: nav_meta
            .referenced_key_column
            .as_ref()
            .map(|s| s.to_string()),
    })
}

/// Loads related entities and fills navigation properties on `entities`.
///
/// `filter_map` carries per-table query filters (e.g. tenant isolation). When
/// present, the filter for each related table is appended (AND'd) to the
/// secondary SELECT so navigation data is scoped consistently with primary
/// queries.
pub async fn load_includes<T>(
    entities: &mut [T],
    includes: &[IncludePath],
    provider: &dyn IDatabaseProvider,
    filter_map: Option<&HashMap<String, CompiledFilter>>,
) -> EFResult<()>
where
    T: IEntityType + IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
{
    if entities.is_empty() || includes.is_empty() {
        return Ok(());
    }

    let meta = T::entity_meta();
    for include in includes {
        let nav = match meta.find_navigation(&include.navigation) {
            Some(n) => n,
            None => continue,
        };

        if nav.kind == NavigationKind::ManyToMany {
            load_many_to_many(entities, &include.navigation, nav, provider, filter_map).await?;
        } else {
            load_scalar_navigation(entities, include, nav, &meta, provider, filter_map).await?;
        }

        if !include.nested.is_empty() {
            for entity in entities.iter_mut() {
                entity
                    .load_nested_includes(
                        &include.navigation,
                        &include.nested,
                        provider,
                        filter_map,
                    )
                    .await?;
            }
        }
    }
    Ok(())
}

async fn load_scalar_navigation<T>(
    entities: &mut [T],
    include: &IncludePath,
    nav: &NavigationMeta,
    meta: &EntityTypeMeta,
    provider: &dyn IDatabaseProvider,
    filter_map: Option<&HashMap<String, CompiledFilter>>,
) -> EFResult<()>
where
    T: IEntitySnapshot + INavigationSetter + IGetKeyValues,
{
    let related_table = include
        .related_table
        .as_deref()
        .or(nav.related_table.as_deref());
    let fk_column = include
        .foreign_key_column
        .as_deref()
        .or(nav.fk_column.as_deref());
    let ref_column = include
        .referenced_key_column
        .as_deref()
        .or(nav.referenced_key_column.as_deref())
        .or_else(|| meta.primary_keys.first().map(|k| k.as_ref()));

    let (Some(related_table), Some(fk_column), Some(ref_column)) =
        (related_table, fk_column, ref_column)
    else {
        return Ok(());
    };

    match nav.kind {
        NavigationKind::HasMany => {
            let parent_ids: Vec<DbValue> = entities
                .iter()
                .filter_map(|e| e.key_values().get(ref_column).cloned())
                .collect();
            if parent_ids.is_empty() {
                return Ok(());
            }

            let gen = provider.sql_generator();
            let placeholders: Vec<String> = (0..parent_ids.len())
                .map(|i| gen.parameter_placeholder(i + 1))
                .collect();
            let mut sql = format!(
                "SELECT * FROM {} WHERE {} IN ({})",
                related_table,
                fk_column,
                placeholders.join(", ")
            );
            let mut params = parent_ids;
            apply_filter_to_sql(&mut sql, &mut params, related_table, filter_map, gen);

            let mut conn = provider.get_connection().await?;
            let rows = conn.query(&sql, &params).await?;
            let grouped = group_rows(&rows, nav.fk_row_index);

            for entity in entities.iter_mut() {
                if let Some(pk) = entity.key_values().get(ref_column) {
                    let key = db_value_key(pk);
                    if let Some(child_rows) = grouped.get(&key) {
                        entity.apply_has_many(&include.navigation, child_rows)?;
                    }
                }
            }
        }
        NavigationKind::BelongsTo | NavigationKind::HasOne => {
            let fk_values: Vec<DbValue> = entities
                .iter()
                .filter_map(|e| e.snapshot().get(fk_column).cloned())
                .collect();
            if fk_values.is_empty() {
                return Ok(());
            }

            let gen = provider.sql_generator();
            let placeholders: Vec<String> = (0..fk_values.len())
                .map(|i| gen.parameter_placeholder(i + 1))
                .collect();
            let mut sql = format!(
                "SELECT * FROM {} WHERE {} IN ({})",
                related_table,
                ref_column,
                placeholders.join(", ")
            );
            let mut params = fk_values;
            apply_filter_to_sql(&mut sql, &mut params, related_table, filter_map, gen);

            let mut conn = provider.get_connection().await?;
            let rows = conn.query(&sql, &params).await?;
            let lookup = index_rows(&rows, nav.pk_row_index);

            for entity in entities.iter_mut() {
                if let Some(fk) = entity.snapshot().get(fk_column) {
                    let key = db_value_key(fk);
                    if let Some(related_row) = lookup.get(&key) {
                        entity.apply_reference(&include.navigation, related_row)?;
                    }
                }
            }
        }
        NavigationKind::ManyToMany => {
            unreachable!("many-to-many navigations are loaded via load_many_to_many")
        }
    }
    Ok(())
}

async fn load_many_to_many<T>(
    entities: &mut [T],
    navigation: &str,
    nav: &NavigationMeta,
    provider: &dyn IDatabaseProvider,
    filter_map: Option<&HashMap<String, CompiledFilter>>,
) -> EFResult<()>
where
    T: IEntityType + INavigationSetter + IGetKeyValues,
{
    let meta = T::entity_meta();
    let ref_column = meta
        .primary_keys
        .first()
        .map(|k| k.as_ref())
        .unwrap_or("id");

    let (Some(join_table), Some(parent_fk), Some(_related_fk), Some(related_table)) = (
        nav.through_table.as_deref(),
        nav.through_parent_fk.as_deref(),
        nav.through_related_fk.as_deref(),
        nav.related_table.as_deref(),
    ) else {
        return Ok(());
    };

    let parent_ids: Vec<DbValue> = entities
        .iter()
        .filter_map(|e| e.key_values().get(ref_column).cloned())
        .collect();
    if parent_ids.is_empty() {
        return Ok(());
    }

    let gen = provider.sql_generator();
    let placeholders: Vec<String> = (0..parent_ids.len())
        .map(|i| gen.parameter_placeholder(i + 1))
        .collect();
    let join_sql = format!(
        "SELECT * FROM {} WHERE {} IN ({})",
        join_table,
        parent_fk,
        placeholders.join(", ")
    );

    let mut conn = provider.get_connection().await?;
    let join_rows = conn.query(&join_sql, &parent_ids).await?;
    if join_rows.is_empty() {
        return Ok(());
    }

    let parent_to_related = group_join_rows(
        &join_rows,
        nav.through_parent_fk_index,
        nav.through_related_fk_index,
    );

    let pk_type_id = nav
        .related_entity_meta
        .and_then(|meta_fn| {
            meta_fn()
                .properties
                .iter()
                .find(|p| p.is_primary_key)
                .map(|p| p.type_id)
        })
        .unwrap_or_else(TypeId::of::<i32>);

    // Collect unique related keys using a HashSet for O(1) dedup
    // (previously O(N²) linear scan per join row).
    let mut seen: HashSet<String> = HashSet::new();
    let related_keys: Vec<String> = join_rows
        .iter()
        .filter_map(|row| row.get(nav.through_related_fk_index))
        .filter(|v| seen.insert((*v).clone()))
        .cloned()
        .collect();

    if related_keys.is_empty() {
        return Ok(());
    }

    let related_ids: Vec<DbValue> = related_keys
        .iter()
        .map(|s| cell_to_db_value(s, pk_type_id))
        .collect();

    let related_pk = nav.referenced_key_column.as_deref().unwrap_or("id");
    let rel_placeholders: Vec<String> = (0..related_ids.len())
        .map(|i| gen.parameter_placeholder(i + 1))
        .collect();
    let mut related_sql = format!(
        "SELECT * FROM {} WHERE {} IN ({})",
        related_table,
        related_pk,
        rel_placeholders.join(", ")
    );
    let mut related_params = related_ids;
    apply_filter_to_sql(
        &mut related_sql,
        &mut related_params,
        related_table,
        filter_map,
        gen,
    );
    let related_rows = conn.query(&related_sql, &related_params).await?;
    let related_by_pk = index_rows(&related_rows, nav.pk_row_index);

    for entity in entities.iter_mut() {
        if let Some(pk) = entity.key_values().get(ref_column) {
            let parent_key = db_value_key(pk);
            if let Some(related_keys) = parent_to_related.get(&parent_key) {
                let mut child_rows = Vec::new();
                for rk in related_keys {
                    if let Some(row) = related_by_pk.get(rk) {
                        child_rows.push(row.clone());
                    }
                }
                if !child_rows.is_empty() {
                    entity.apply_has_many(navigation, &child_rows)?;
                }
            }
        }
    }

    Ok(())
}

fn db_value_key(v: &DbValue) -> String {
    format!("{}", v)
}

fn group_rows(rows: &[Vec<String>], fk_index: usize) -> HashMap<String, Vec<Vec<String>>> {
    let mut map: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    for row in rows {
        if let Some(fk_val) = row.get(fk_index) {
            map.entry(fk_val.clone()).or_default().push(row.clone());
        }
    }
    map
}

fn group_join_rows(
    rows: &[Vec<String>],
    parent_index: usize,
    related_index: usize,
) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        if let (Some(parent), Some(related)) = (row.get(parent_index), row.get(related_index)) {
            map.entry(parent.clone()).or_default().push(related.clone());
        }
    }
    map
}

fn index_rows(rows: &[Vec<String>], pk_index: usize) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::new();
    for row in rows {
        if let Some(pk) = row.get(pk_index) {
            map.insert(pk.clone(), row.clone());
        }
    }
    map
}

fn cell_to_db_value(raw: &str, type_id: TypeId) -> DbValue {
    if type_id == TypeId::of::<i32>() {
        raw.parse::<i32>()
            .map(DbValue::I32)
            .unwrap_or_else(|_| DbValue::String(raw.to_string()))
    } else if type_id == TypeId::of::<i64>() {
        raw.parse::<i64>()
            .map(DbValue::I64)
            .unwrap_or_else(|_| DbValue::String(raw.to_string()))
    } else if type_id == TypeId::of::<i16>() {
        raw.parse::<i16>()
            .map(DbValue::I16)
            .unwrap_or_else(|_| DbValue::String(raw.to_string()))
    } else if type_id == TypeId::of::<f64>() {
        raw.parse::<f64>()
            .map(DbValue::F64)
            .unwrap_or_else(|_| DbValue::String(raw.to_string()))
    } else if type_id == TypeId::of::<f32>() {
        raw.parse::<f32>()
            .map(DbValue::F32)
            .unwrap_or_else(|_| DbValue::String(raw.to_string()))
    } else if type_id == TypeId::of::<bool>() {
        raw.parse::<bool>()
            .map(DbValue::Bool)
            .unwrap_or_else(|_| DbValue::String(raw.to_string()))
    } else {
        DbValue::String(raw.to_string())
    }
}
