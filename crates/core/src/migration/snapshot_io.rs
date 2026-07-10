//! Model snapshot JSON serialization / deserialization.

use super::types::{ModelSnapshot, SnapshotColumn, SnapshotEntityType};
use crate::error::{EFError, EFResult};

/// Parses a model snapshot JSON file (same format as [`super::history::MigrationStore::save_snapshot`]).
pub fn parse_model_snapshot_json(text: &str) -> EFResult<Option<ModelSnapshot>> {
    snapshot_from_json(text)
}

pub(super) fn migration_io_err(e: std::io::Error) -> EFError {
    EFError::migration(e.to_string())
}

pub(super) fn snapshot_to_json(snapshot: &ModelSnapshot) -> String {
    let mut out = String::from("{\n  \"migration_id\": \"");
    out.push_str(&snapshot.migration_id.replace('"', "\\\""));
    out.push_str("\",\n  \"entity_types\": [\n");
    for (i, et) in snapshot.entity_types.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"type_name\": \"{}\",\n",
            et.type_name.replace('"', "\\\"")
        ));
        out.push_str(&format!(
            "      \"table_name\": \"{}\",\n",
            et.table_name.replace('"', "\\\"")
        ));
        out.push_str("      \"columns\": [\n");
        for (j, col) in et.columns.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "        {{\"field_name\":\"{}\",\"column_name\":\"{}\",\"type_name\":\"{}\",\"is_primary_key\":{},\"is_required\":{},\"is_foreign_key\":{},\"max_length\":{},\"is_auto_increment\":{},\"is_sequence\":{},\"sequence_name\":{},\"fk_referenced_table\":{},\"fk_referenced_column\":{},\"has_index\":{},\"is_unique\":{},\"fk_on_delete\":{}}}\n",
                col.field_name.replace('"', "\\\""),
                col.column_name.replace('"', "\\\""),
                col.type_name.replace('"', "\\\""),
                col.is_primary_key,
                col.is_required,
                col.is_foreign_key,
                col.max_length.map(|n| n.to_string()).unwrap_or_else(|| "null".into()),
                col.is_auto_increment,
                col.is_sequence,
                col.sequence_name
                    .as_ref()
                    .map(|s| format!("\"{}\"", s.replace('"', "\\\"")))
                    .unwrap_or_else(|| "null".into()),
                col.fk_referenced_table
                    .as_ref()
                    .map(|s| format!("\"{}\"", s.replace('"', "\\\"")))
                    .unwrap_or_else(|| "null".into()),
                col.fk_referenced_column
                    .as_ref()
                    .map(|s| format!("\"{}\"", s.replace('"', "\\\"")))
                    .unwrap_or_else(|| "null".into()),
                col.has_index,
                col.is_unique,
                col.fk_on_delete
                    .as_ref()
                    .map(|s| format!("\"{}\"", s.replace('"', "\\\"")))
                    .unwrap_or_else(|| "null".into())
            ));
        }
        out.push_str("      ]\n    }\n");
    }
    out.push_str("  ]\n}\n");
    out
}

fn snapshot_from_json(text: &str) -> EFResult<Option<ModelSnapshot>> {
    // Minimal JSON parser for snapshot files written by save_snapshot.
    let migration_id =
        extract_json_string(text, "migration_id").unwrap_or_else(|| "__snapshot__".to_string());
    let mut entity_types = Vec::new();
    if let Some(arr_start) = text.find("\"entity_types\"") {
        let slice = &text[arr_start..];
        for table_block in slice.split("\"table_name\"").skip(1) {
            let table_name = extract_quoted_after_colon(table_block).unwrap_or_default();
            let _type_name = entity_types.len().to_string(); // fallback
            let columns_start = table_block.find("\"columns\"");
            let mut columns = Vec::new();
            if let Some(cs) = columns_start {
                let col_slice = &table_block[cs..];
                for col_chunk in col_slice.split("\"field_name\"").skip(1) {
                    if let Some(field_name) = extract_quoted_after_colon(col_chunk) {
                        let column_name = extract_json_string(col_chunk, "column_name")
                            .unwrap_or_else(|| field_name.clone());
                        let type_name_col = extract_json_string(col_chunk, "type_name")
                            .unwrap_or_else(|| "String".to_string());
                        columns.push(SnapshotColumn {
                            field_name,
                            column_name,
                            type_name: type_name_col,
                            is_primary_key: col_chunk.contains("\"is_primary_key\":true"),
                            is_required: col_chunk.contains("\"is_required\":true"),
                            is_foreign_key: col_chunk.contains("\"is_foreign_key\":true"),
                            max_length: None,
                            is_auto_increment: col_chunk.contains("\"is_auto_increment\":true"),
                            is_sequence: col_chunk.contains("\"is_sequence\":true"),
                            sequence_name: extract_json_string(col_chunk, "sequence_name"),
                            fk_referenced_table: extract_json_string(
                                col_chunk,
                                "fk_referenced_table",
                            ),
                            fk_referenced_column: extract_json_string(
                                col_chunk,
                                "fk_referenced_column",
                            ),
                            has_index: col_chunk.contains("\"has_index\":true"),
                            is_unique: col_chunk.contains("\"is_unique\":true"),
                            fk_on_delete: extract_json_string(col_chunk, "fk_on_delete"),
                        });
                    }
                }
            }
            if !table_name.is_empty() {
                entity_types.push(SnapshotEntityType {
                    type_name: table_name.clone(),
                    table_name,
                    columns,
                });
            }
        }
    }
    if entity_types.is_empty() {
        return Ok(None);
    }
    Ok(Some(ModelSnapshot {
        migration_id,
        entity_types,
    }))
}

fn extract_json_string(haystack: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let pos = haystack.find(&needle)?;
    extract_quoted_after_colon(&haystack[pos + needle.len()..])
}

fn extract_quoted_after_colon(s: &str) -> Option<String> {
    let colon = s.find(':')?;
    let rest = s[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}
