//! Model snapshot JSON serialization / deserialization.

use super::types::{
    ModelSnapshot, SnapshotCheck, SnapshotColumn, SnapshotEntityType, SnapshotForeignKey,
    SnapshotIndex, SnapshotIndexColumn, SnapshotTableOptions,
};
use crate::error::{EFError, EFResult};
use serde_json::{json, Map, Value};

/// Parses a model snapshot JSON file (same format as
/// [`super::history::MigrationStore::save_snapshot`]).
pub fn parse_model_snapshot_json(text: &str) -> EFResult<Option<ModelSnapshot>> {
    snapshot_from_json(text)
}

pub(super) fn migration_io_err(e: std::io::Error) -> EFError {
    EFError::migration(e.to_string())
}

pub(super) fn snapshot_to_json(snapshot: &ModelSnapshot) -> String {
    let entity_types: Vec<Value> = snapshot
        .entity_types
        .iter()
        .map(|et| {
            let columns: Vec<Value> = et
                .columns
                .iter()
                .map(|c| {
                    json!({
                        "field_name": c.field_name,
                        "column_name": c.column_name,
                        "type_name": c.type_name,
                        "is_primary_key": c.is_primary_key,
                        "is_required": c.is_required,
                        "is_foreign_key": c.is_foreign_key,
                        "max_length": c.max_length,
                        "is_auto_increment": c.is_auto_increment,
                        "is_sequence": c.is_sequence,
                        "sequence_name": c.sequence_name,
                        "fk_referenced_table": c.fk_referenced_table,
                        "fk_referenced_column": c.fk_referenced_column,
                        "has_index": c.has_index,
                        "is_unique": c.is_unique,
                        "fk_on_delete": c.fk_on_delete,
                        "default_value": c.default_value,
                        "column_type": c.column_type,
                        "precision": c.precision,
                        "scale": c.scale,
                        "is_fixed_length": c.is_fixed_length,
                        "collation": c.collation,
                        "comment": c.comment,
                        "computed_sql": c.computed_sql,
                        "computed_stored": c.computed_stored,
                    })
                })
                .collect();
            let indexes: Vec<Value> = et
                .indexes
                .iter()
                .map(|i| {
                    json!({
                        "name": i.name,
                        "columns": i
                            .columns
                            .iter()
                            .map(|c| json!({"value": c.value, "is_expression": c.is_expression}))
                            .collect::<Vec<_>>(),
                        "is_unique": i.is_unique,
                        "filter": i.filter,
                        "include": i.include,
                    })
                })
                .collect();
            let foreign_keys: Vec<Value> = et
                .foreign_keys
                .iter()
                .map(|f| {
                    json!({
                        "name": f.name,
                        "columns": f.columns,
                        "referenced_table": f.referenced_table,
                        "referenced_columns": f.referenced_columns,
                        "on_delete": f.on_delete,
                    })
                })
                .collect();
            let checks: Vec<Value> = et
                .checks
                .iter()
                .map(|c| json!({"name": c.name, "expression": c.expression}))
                .collect();
            json!({
                "type_name": et.type_name,
                "table_name": et.table_name,
                "columns": columns,
                "indexes": indexes,
                "foreign_keys": foreign_keys,
                "checks": checks,
                "table_options": {
                    "engine": et.table_options.engine,
                    "charset": et.table_options.charset,
                    "collate": et.table_options.collate,
                },
                "is_view": et.is_view,
                "is_keyless": et.is_keyless,
                "comment": et.comment,
            })
        })
        .collect();

    serde_json::to_string_pretty(&json!({
        "migration_id": snapshot.migration_id,
        "entity_types": entity_types,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn snapshot_from_json(text: &str) -> EFResult<Option<ModelSnapshot>> {
    let root: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let Some(entity_types) = root.get("entity_types").and_then(Value::as_array) else {
        return Ok(None);
    };

    let migration_id = root
        .get("migration_id")
        .and_then(Value::as_str)
        .unwrap_or("__snapshot__")
        .to_string();

    let mut types = Vec::with_capacity(entity_types.len());
    for et in entity_types {
        let Some(obj) = et.as_object() else {
            continue;
        };
        let table_name = string_field(obj, "table_name");
        if table_name.is_empty() {
            continue;
        }
        let columns = obj
            .get("columns")
            .and_then(Value::as_array)
            .map(|cols| {
                cols.iter()
                    .filter_map(Value::as_object)
                    .map(|c| SnapshotColumn {
                        field_name: string_field(c, "field_name"),
                        column_name: {
                            let name = string_field(c, "column_name");
                            if name.is_empty() {
                                string_field(c, "field_name")
                            } else {
                                name
                            }
                        },
                        type_name: {
                            let name = string_field(c, "type_name");
                            if name.is_empty() {
                                "String".to_string()
                            } else {
                                name
                            }
                        },
                        is_primary_key: bool_field(c, "is_primary_key"),
                        is_required: bool_field(c, "is_required"),
                        is_foreign_key: bool_field(c, "is_foreign_key"),
                        max_length: c
                            .get("max_length")
                            .and_then(Value::as_u64)
                            .map(|n| n as usize),
                        is_auto_increment: bool_field(c, "is_auto_increment"),
                        is_sequence: bool_field(c, "is_sequence"),
                        sequence_name: opt_string(c, "sequence_name"),
                        fk_referenced_table: opt_string(c, "fk_referenced_table"),
                        fk_referenced_column: opt_string(c, "fk_referenced_column"),
                        has_index: bool_field(c, "has_index"),
                        is_unique: bool_field(c, "is_unique"),
                        fk_on_delete: opt_string(c, "fk_on_delete"),
                        default_value: opt_string(c, "default_value"),
                        column_type: opt_string(c, "column_type"),
                        precision: c.get("precision").and_then(Value::as_u64).map(|n| n as u32),
                        scale: c.get("scale").and_then(Value::as_u64).map(|n| n as u32),
                        is_fixed_length: bool_field(c, "is_fixed_length"),
                        collation: opt_string(c, "collation"),
                        comment: opt_string(c, "comment"),
                        computed_sql: opt_string(c, "computed_sql"),
                        computed_stored: bool_field(c, "computed_stored"),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let indexes = obj
            .get("indexes")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|i| SnapshotIndex {
                        name: string_field(i, "name"),
                        columns: i
                            .get("columns")
                            .and_then(Value::as_array)
                            .map(|cols| {
                                cols.iter()
                                    .filter_map(Value::as_object)
                                    .map(|c| SnapshotIndexColumn {
                                        value: string_field(c, "value"),
                                        is_expression: bool_field(c, "is_expression"),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        is_unique: bool_field(i, "is_unique"),
                        filter: opt_string(i, "filter"),
                        include: string_array(i, "include"),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let foreign_keys = obj
            .get("foreign_keys")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|f| SnapshotForeignKey {
                        name: string_field(f, "name"),
                        columns: string_array(f, "columns"),
                        referenced_table: string_field(f, "referenced_table"),
                        referenced_columns: string_array(f, "referenced_columns"),
                        on_delete: opt_string(f, "on_delete"),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let checks = obj
            .get("checks")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|c| SnapshotCheck {
                        name: string_field(c, "name"),
                        expression: string_field(c, "expression"),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let table_options = obj
            .get("table_options")
            .and_then(Value::as_object)
            .map(|o| SnapshotTableOptions {
                engine: opt_string(o, "engine"),
                charset: opt_string(o, "charset"),
                collate: opt_string(o, "collate"),
            })
            .unwrap_or_default();

        types.push(SnapshotEntityType {
            type_name: {
                let name = string_field(obj, "type_name");
                if name.is_empty() {
                    table_name.clone()
                } else {
                    name
                }
            },
            table_name,
            columns,
            indexes,
            foreign_keys,
            checks,
            table_options,
            is_view: bool_field(obj, "is_view"),
            is_keyless: bool_field(obj, "is_keyless"),
            comment: opt_string(obj, "comment"),
        });
    }

    if types.is_empty() {
        return Ok(None);
    }
    Ok(Some(ModelSnapshot {
        migration_id,
        entity_types: types,
    }))
}

fn string_field(obj: &Map<String, Value>, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn opt_string(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_string)
}

fn bool_field(obj: &Map<String, Value>, key: &str) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn string_array(obj: &Map<String, Value>, key: &str) -> Vec<String> {
    obj.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
