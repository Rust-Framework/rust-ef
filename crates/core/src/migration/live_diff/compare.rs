//! Model-vs-live-schema detection.
//!
//! [`detect`] produces one [`TableDelta`] per model table plus structured
//! warnings. It is dialect-aware only where comparison semantics require it
//! (type/default normalization); rendering and rebuild decisions live in the
//! sibling `render` module.

use crate::schema::{DbCheck, DbColumn, DbIndex, DbTable};

use crate::migration::dialect::MigrationDialect;
use crate::migration::diff::fk_target;
use crate::migration::engine::MigrationEngine;
use crate::migration::schema_change::SchemaChange;
use crate::migration::types::{
    ModelSnapshot, SnapshotCheck, SnapshotColumn, SnapshotEntityType, SnapshotForeignKey,
    SnapshotIndex, SnapshotIndexColumn,
};
use super::{DiffWarning, WarningKind};

/// Detected work for one model table.
pub(crate) struct TableDelta<'a> {
    /// Table name.
    pub table: String,
    /// Model-side definition.
    pub model: Option<&'a SnapshotEntityType>,
    /// Live-side definition (`None` = table must be created).
    pub live: Option<&'a DbTable>,
    /// Logical changes, in detection order.
    pub changes: Vec<SchemaChange>,
    /// Reasons the table must be rebuilt on SQLite (empty = native ALTER path).
    pub rebuild_reasons: Vec<String>,
}

/// Compares every model table with the live schema.
pub(crate) fn detect<'a>(
    model: &'a ModelSnapshot,
    live: &'a [DbTable],
    dialect: MigrationDialect,
) -> (Vec<TableDelta<'a>>, Vec<DiffWarning>) {
    let mut live_by_name: std::collections::HashMap<&str, &DbTable> =
        std::collections::HashMap::with_capacity(live.len());
    for table in live {
        live_by_name.insert(table.name.as_str(), table);
    }

    let mut deltas = Vec::new();
    let mut warnings = Vec::new();

    for et in model.entity_types.iter().filter(|et| !et.is_view) {
        match find_table_ci(live, &et.table_name) {
        None => {
            // Unsupported index definitions are reported even for tables that
            // are created from scratch, so the operator sees them in the plan
            // instead of only as a comment inside the generated SQL.
            for index in &et.indexes {
                match index_support(dialect, &et.table_name, index) {
                    IndexSupport::IgnoredInclude(warning) | IndexSupport::Skipped(warning) => {
                        warnings.push(warning)
                    }
                    IndexSupport::Supported => {}
                }
            }
            deltas.push(TableDelta {
                table: et.table_name.clone(),
                model: Some(et),
                live: None,
                changes: Vec::new(),
                rebuild_reasons: Vec::new(),
            })
        }
            Some(live_table) => {
                let delta = compare_table(et, live_table, dialect, &mut warnings);
                deltas.push(delta);
            }
        }
    }

    // Live tables that the model does not know about. Framework bookkeeping
    // tables (`__ef_*`) and database-internal tables (`sqlite_*`) are ignored.
    let model_tables: std::collections::HashSet<String> = model
        .entity_types
        .iter()
        .filter(|et| !et.is_view)
        .map(|et| et.table_name.to_ascii_lowercase())
        .collect();
    for table in live {
        if model_tables.contains(&table.name.to_ascii_lowercase())
            || is_framework_table(&table.name)
        {
            continue;
        }
        warnings.push(DiffWarning::info(
            WarningKind::SkippedRemoval,
            Some(table.name.clone()),
            format!(
                "table '{}' exists in the live database but is not declared in the model; \
                 runtime diff never drops tables (apply a versioned migration if removal is intended)",
                table.name
            ),
        ));
    }

    (deltas, warnings)
}

/// Framework/internal tables are not part of the model and are never reported.
fn is_framework_table(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("__ef_") || name.starts_with("sqlite_")
}

fn find_table_ci<'a>(live: &'a [DbTable], name: &str) -> Option<&'a DbTable> {
    live.iter().find(|table| table.name.eq_ignore_ascii_case(name))
}

fn compare_table<'a>(
    et: &'a SnapshotEntityType,
    live: &'a DbTable,
    dialect: MigrationDialect,
    warnings: &mut Vec<DiffWarning>,
) -> TableDelta<'a> {
    let mut delta = TableDelta {
        table: et.table_name.clone(),
        model: Some(et),
        live: Some(live),
        changes: Vec::new(),
        rebuild_reasons: Vec::new(),
    };

    compare_primary_key(&mut delta, et, live, dialect, warnings);
    compare_columns(&mut delta, et, live, dialect, warnings);
    compare_indexes(&mut delta, et, live, dialect, warnings);
    compare_foreign_keys(&mut delta, et, live, warnings);
    compare_checks(&mut delta, et, live, warnings);

    delta
}

// ---------------------------------------------------------------------------
// Primary key
// ---------------------------------------------------------------------------

fn primary_key_columns(columns: &[SnapshotColumn]) -> Vec<String> {
    columns
        .iter()
        .filter(|c| c.is_primary_key)
        .map(|c| c.column_name.clone())
        .collect()
}

fn compare_primary_key(
    delta: &mut TableDelta<'_>,
    et: &SnapshotEntityType,
    live: &DbTable,
    dialect: MigrationDialect,
    warnings: &mut Vec<DiffWarning>,
) {
    let model_pk = primary_key_columns(&et.columns);
    let live_pk: Vec<String> = live
        .columns
        .iter()
        .filter(|c| c.is_primary_key)
        .map(|c| c.name.clone())
        .collect();
    if eq_ci_slices(&model_pk, &live_pk) {
        return;
    }
    delta.rebuild_reasons.push(format!(
        "primary key changes from ({}) to ({})",
        live_pk.join(", "),
        model_pk.join(", ")
    ));
    if dialect != MigrationDialect::Sqlite {
        let table = &delta.table;
        warnings.push(DiffWarning::warning(
            WarningKind::ManualAction,
            Some(table.clone()),
            format!(
                "primary key of table '{table}' changes from ({}) to ({}); \
                 changing a primary key on an existing table is not applied automatically",
                live_pk.join(", "),
                model_pk.join(", ")
            ),
        ));
    }
}

// ---------------------------------------------------------------------------
// Columns
// ---------------------------------------------------------------------------

fn find_column_ci<'a>(live: &'a DbTable, name: &str) -> Option<&'a DbColumn> {
    live.columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(name))
}

fn compare_columns(
    delta: &mut TableDelta<'_>,
    et: &SnapshotEntityType,
    live: &DbTable,
    dialect: MigrationDialect,
    warnings: &mut Vec<DiffWarning>,
) {
    let table = delta.table.clone();
    for column in &et.columns {
        match find_column_ci(live, &column.column_name) {
            None => {
                delta.changes.push(SchemaChange::AddColumn {
                    table: table.clone(),
                    column: column.clone(),
                });
                if column.is_required
                    && !column.is_primary_key
                    && column.default_value.is_none()
                    && column.computed_sql.is_none()
                {
                    warnings.push(DiffWarning::warning(
                        WarningKind::NotNullColumnWithoutDefault,
                        Some(table.clone()),
                        format!(
                            "adding NOT NULL column '{}' to '{}' without a default fails on a non-empty table; \
                             configure a default (has_default) or backfill the column first",
                            column.column_name, table
                        ),
                    ));
                }
            }
            Some(live_column) => {
                if !column_matches(dialect, column, live_column) {
                    // PostgreSQL cannot change a generated-column expression;
                    // the DDL generator only emits a warning comment, which the
                    // runtime plan must surface as a structured warning.
                    if dialect == MigrationDialect::Postgres && column.computed_sql.is_some() {
                        warnings.push(DiffWarning::warning(
                            WarningKind::ManualAction,
                            Some(table.clone()),
                            format!(
                                "PostgreSQL cannot alter a generated column expression; \
                                 drop and re-add column '{}.{}' manually",
                                table, column.column_name
                            ),
                        ));
                        continue;
                    }
                    let reason = describe_column_difference(dialect, column, live_column);
                    delta.rebuild_reasons.push(reason);
                    delta.changes.push(SchemaChange::AlterColumn {
                        table: table.clone(),
                        column_name: column.column_name.clone(),
                        old: Box::new(snapshot_column_from_live(live_column)),
                        new: Box::new(column.clone()),
                    });
                }
            }
        }
    }

    for live_column in &live.columns {
        if et.columns
            .iter()
            .any(|c| c.column_name.eq_ignore_ascii_case(&live_column.name))
        {
            continue;
        }
        warnings.push(DiffWarning::info(
            WarningKind::SkippedRemoval,
            Some(table.clone()),
            format!(
                "column '{}.{}' exists in the live database but not in the model; \
                 runtime diff never drops columns (column renames appear as add + drop)",
                table, live_column.name
            ),
        ));
    }
}

/// Structural comparison between a model column and the live column.
///
/// Index state (`has_index`/`is_unique`) is intentionally excluded: index
/// changes are planned separately so they produce `CREATE INDEX`, not `ALTER`.
fn column_matches(dialect: MigrationDialect, model: &SnapshotColumn, live: &DbColumn) -> bool {
    let model_not_null = model.is_required || model.is_primary_key;
    if model_not_null != live.is_not_null() {
        return false;
    }
    if !column_types_match(dialect, model, live) {
        return false;
    }
    match (&model.default_value, &live.default_value) {
        (None, None) => true,
        // A missing default is a real difference on every dialect.
        (Some(_), None) => false,
        // Server-generated implicit defaults (e.g. PostgreSQL `nextval`) are not
        // part of the model; do not flag them.
        (None, Some(_)) => dialect == MigrationDialect::Sqlite,
        // Default text is normalized by the server on PostgreSQL/MySQL; only
        // SQLite reports the original literal verbatim (`PRAGMA table_info`).
        (Some(model_default), Some(live_default)) => {
            dialect != MigrationDialect::Sqlite
                || normalize_default(model_default) == normalize_default(live_default)
        }
    }
}

fn describe_column_difference(
    dialect: MigrationDialect,
    model: &SnapshotColumn,
    live: &DbColumn,
) -> String {
    let model_not_null = model.is_required || model.is_primary_key;
    let mut parts = Vec::new();
    if model_not_null != live.is_not_null() {
        parts.push(format!(
            "nullability {} -> {}",
            if live.is_not_null() { "NOT NULL" } else { "NULL" },
            if model_not_null { "NOT NULL" } else { "NULL" }
        ));
    }
    if !column_types_match(dialect, model, live) {
        parts.push(format!(
            "type '{}' -> '{}'",
            live.data_type,
            dialect.map_column_type(model)
        ));
    }
    if let (Some(model_default), Some(live_default)) = (&model.default_value, &live.default_value) {
        if dialect == MigrationDialect::Sqlite
            && normalize_default(model_default) != normalize_default(live_default)
        {
            parts.push(format!("default {live_default} -> {model_default}"));
        }
    }
    if parts.is_empty() {
        parts.push("column definition changed".to_string());
    }
    format!("column '{}': {}", model.column_name, parts.join(", "))
}

/// Builds a best-effort snapshot column from live introspection (used only for
/// the `old` side of [`SchemaChange::AlterColumn`], i.e. for Down scripts).
fn snapshot_column_from_live(live: &DbColumn) -> SnapshotColumn {
    SnapshotColumn {
        field_name: live.name.clone(),
        column_name: live.name.clone(),
        type_name: live.data_type.clone(),
        is_primary_key: live.is_primary_key,
        is_required: live.is_not_null(),
        is_auto_increment: live.is_auto_increment,
        max_length: live.max_length,
        default_value: live.default_value.clone(),
        precision: live.precision,
        scale: live.scale,
        ..SnapshotColumn::default()
    }
}

// ---------------------------------------------------------------------------
// Type / default normalization
// ---------------------------------------------------------------------------

fn column_types_match(dialect: MigrationDialect, model: &SnapshotColumn, live: &DbColumn) -> bool {
    // Providers that do not report a type at all cannot be verified; assume a
    // match instead of generating a spurious ALTER.
    if live.data_type.trim().is_empty() {
        return true;
    }
    let expected = dialect.map_column_type(model);
    match dialect {
        MigrationDialect::Sqlite => {
            normalize_sqlite_type(&expected) == normalize_sqlite_type(&live.data_type)
        }
        MigrationDialect::Postgres => pg_type(&expected, model) == pg_live_type(live),
        MigrationDialect::MySql => mysql_type(&expected) == mysql_type(&live.data_type),
    }
}

fn normalize_ws(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// SQLite type names are the declared type text, but `INT` and `INTEGER` are
/// the same affinity and the generator emits `INTEGER`; collapse that synonym.
fn normalize_sqlite_type(value: &str) -> String {
    let normalized = normalize_ws(value);
    if normalized == "int" {
        "integer".to_string()
    } else {
        normalized
    }
}

/// Canonical PostgreSQL type name for a model-mapped type.
fn pg_type(expected: &str, model: &SnapshotColumn) -> String {
    let normalized = pg_alias(&normalize_ws(expected));
    let normalized = append_numeric_precision(&normalized, model.precision, model.scale);
    append_length(&normalized, model.max_length)
}

fn pg_alias(value: &str) -> String {
    match value {
        "serial" => "integer".to_string(),
        "bigserial" => "bigint".to_string(),
        "smallserial" => "smallint".to_string(),
        "int" | "int4" => "integer".to_string(),
        "int8" => "bigint".to_string(),
        "int2" => "smallint".to_string(),
        "bool" => "boolean".to_string(),
        "float8" => "double precision".to_string(),
        "float4" => "real".to_string(),
        "character varying" => "varchar".to_string(),
        "character" => "char".to_string(),
        "timestamp with time zone" => "timestamptz".to_string(),
        "timestamp without time zone" => "timestamp".to_string(),
        "decimal" => "numeric".to_string(),
        other => other.to_string(),
    }
}

/// Canonical PostgreSQL type name for introspected live data.
fn pg_live_type(live: &DbColumn) -> String {
    let normalized = pg_alias(&normalize_ws(&live.data_type));
    let normalized = if normalized.contains('(') {
        normalized
    } else {
        append_length(&normalized, live.max_length)
    };
    append_numeric_precision(&normalized, live.precision, live.scale)
}

/// Canonical MySQL type name (information_schema `COLUMN_TYPE` spelling).
fn mysql_type(value: &str) -> String {
    let normalized = normalize_ws(value);
    let normalized = normalized
        .strip_suffix(" auto_increment")
        .unwrap_or(&normalized)
        .trim()
        .to_string();
    normalized
        .replace(" integer", " int")
        .replace("integer", "int")
        .replace("double precision", "double")
        .replace("boolean", "tinyint(1)")
        .replace("bool", "tinyint(1)")
}

fn append_length(type_name: &str, max_length: Option<usize>) -> String {
    match (type_name.contains('('), max_length) {
        (false, Some(length)) => format!("{type_name}({length})"),
        _ => type_name.to_string(),
    }
}

fn append_numeric_precision(
    type_name: &str,
    precision: Option<u32>,
    scale: Option<u32>,
) -> String {
    if type_name.contains('(') || (precision.is_none() && scale.is_none()) {
        return type_name.to_string();
    }
    match (precision, scale) {
        (Some(p), Some(s)) => format!("{type_name}({p},{s})"),
        (Some(p), None) => format!("{type_name}({p})"),
        (None, Some(s)) => format!("{type_name}(0,{s})"),
        (None, None) => type_name.to_string(),
    }
}

fn normalize_default(raw: &str) -> String {
    let mut value = raw.trim().to_string();
    loop {
        if value.starts_with('(') && value.ends_with(')') && value.len() > 1 {
            value = value[1..value.len() - 1].trim().to_string();
        } else {
            break;
        }
    }
    value
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// Indexes
// ---------------------------------------------------------------------------

/// Indexes the model expects: column-level `has_index`/`is_unique` plus
/// declarative indexes (composite / partial / expression / include).
pub(crate) fn expected_indexes(et: &SnapshotEntityType) -> Vec<SnapshotIndex> {
    let mut indexes: Vec<SnapshotIndex> = Vec::new();
    for column in &et.columns {
        if !column.has_index && !column.is_unique {
            continue;
        }
        indexes.push(SnapshotIndex {
            name: MigrationEngine::index_name(&et.table_name, &column.column_name),
            columns: vec![SnapshotIndexColumn {
                value: column.column_name.clone(),
                is_expression: false,
            }],
            is_unique: column.is_unique,
            filter: None,
            include: Vec::new(),
        });
    }
    for index in &et.indexes {
        if indexes.iter().any(|existing| existing.name == index.name) {
            continue;
        }
        indexes.push(index.clone());
    }
    indexes
}

/// How well a dialect can represent an expected index.
enum IndexSupport {
    Supported,
    /// Created without the unsupported part (`INCLUDE`), with a warning.
    IgnoredInclude(DiffWarning),
    /// Not created at all (MySQL has no partial non-unique indexes).
    Skipped(DiffWarning),
}

fn index_support(
    dialect: MigrationDialect,
    table: &str,
    index: &SnapshotIndex,
) -> IndexSupport {
    if dialect == MigrationDialect::MySql && index.filter.is_some() && !index.is_unique {
        return IndexSupport::Skipped(DiffWarning::warning(
            WarningKind::UnsupportedChange,
            Some(table.to_string()),
            format!(
                "index '{}' on '{}' is a partial non-unique index; MySQL has no partial indexes, \
                 so the index is skipped (make it unique or remove the filter)",
                index.name, table
            ),
        ));
    }
    if dialect != MigrationDialect::Postgres && !index.include.is_empty() {
        return IndexSupport::IgnoredInclude(DiffWarning::warning(
            WarningKind::UnsupportedChange,
            Some(table.to_string()),
            format!(
                "index '{}' on '{}' declares INCLUDE columns; only PostgreSQL supports INCLUDE, \
                 so the columns are ignored for this dialect",
                index.name, table
            ),
        ));
    }
    IndexSupport::Supported
}

fn compare_indexes(
    delta: &mut TableDelta<'_>,
    et: &SnapshotEntityType,
    live: &DbTable,
    dialect: MigrationDialect,
    warnings: &mut Vec<DiffWarning>,
) {
    let table = delta.table.clone();
    let expected = expected_indexes(et);
    let mut matched_live = vec![false; live.indexes.len()];

    for index in &expected {
        if let Some(position) = live
            .indexes
            .iter()
            .position(|live_index| live_index.name.eq_ignore_ascii_case(&index.name))
        {
            matched_live[position] = true;
            if !index_matches(index, &live.indexes[position]) {
                warnings.push(DiffWarning::warning(
                    WarningKind::IndexDefinitionMismatch,
                    Some(table.clone()),
                    format!(
                        "index '{}' on '{}' exists with a different definition than the model; \
                         drop and recreate it with a versioned migration",
                        index.name, table
                    ),
                ));
            }
            continue;
        }
        // The same index may exist under a different name (e.g. renamed by
        // hand); treat it as present to avoid creating a duplicate.
        if let Some(position) = live
            .indexes
            .iter()
            .position(|live_index| index_matches(index, live_index))
        {
            matched_live[position] = true;
            continue;
        }
        match index_support(dialect, &table, index) {
            IndexSupport::Supported => delta.changes.push(SchemaChange::CreateDeclaredIndex {
                table: table.clone(),
                index: index.clone(),
            }),
            IndexSupport::IgnoredInclude(warning) => {
                warnings.push(warning);
                delta.changes.push(SchemaChange::CreateDeclaredIndex {
                    table: table.clone(),
                    index: index.clone(),
                });
            }
            IndexSupport::Skipped(warning) => warnings.push(warning),
        }
    }

    let live_pk: Vec<String> = live
        .columns
        .iter()
        .filter(|c| c.is_primary_key)
        .map(|c| c.name.clone())
        .collect();
    for (position, live_index) in live.indexes.iter().enumerate() {
        if matched_live[position] || is_implicit_index(live_index, &live_pk) {
            continue;
        }
        warnings.push(DiffWarning::info(
            WarningKind::SkippedRemoval,
            Some(table.clone()),
            format!(
                "index '{}' on '{}' exists in the live database but not in the model; \
                 runtime diff never drops indexes",
                live_index.name, table
            ),
        ));
    }
}

pub(crate) fn index_matches(expected: &SnapshotIndex, live: &DbIndex) -> bool {
    if expected.is_unique != live.is_unique {
        return false;
    }
    let expected_parts: Vec<(String, bool)> = expected
        .columns
        .iter()
        .map(|part| {
            (
                normalize_index_part(&part.value, part.is_expression),
                part.is_expression,
            )
        })
        .collect();
    let live_parts: Vec<(String, bool)> = live
        .columns
        .iter()
        .map(|part| {
            (
                normalize_index_part(&part.value, part.is_expression),
                part.is_expression,
            )
        })
        .collect();
    if expected_parts != live_parts {
        return false;
    }
    // A provider that could not report the partial predicate must not cause a
    // duplicate index to be created.
    match (&expected.filter, &live.filter) {
        (Some(expected_filter), Some(live_filter)) => {
            normalize_expression(expected_filter) == normalize_expression(live_filter)
        }
        _ => true,
    }
}

fn normalize_index_part(value: &str, is_expression: bool) -> String {
    if is_expression {
        normalize_expression(value)
    } else {
        value.trim().to_ascii_lowercase()
    }
}

/// Primary-key / automatic unique indexes are part of the table definition, not
/// standalone indexes the model has to declare.
pub(crate) fn is_implicit_index(index: &DbIndex, live_pk: &[String]) -> bool {
    if index.name.to_ascii_lowercase().starts_with("sqlite_autoindex_") {
        return true;
    }
    if index.name.eq_ignore_ascii_case("primary") {
        return true;
    }
    let index_columns = index.column_names();
    !live_pk.is_empty()
        && index.is_unique
        && index_columns.len() == live_pk.len()
        && index_columns
            .iter()
            .zip(live_pk.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

// ---------------------------------------------------------------------------
// Foreign keys
// ---------------------------------------------------------------------------

fn compare_foreign_keys(
    delta: &mut TableDelta<'_>,
    et: &SnapshotEntityType,
    live: &DbTable,
    warnings: &mut Vec<DiffWarning>,
) {
    let table = delta.table.clone();
    let mut matched_live = vec![false; live.foreign_keys.len()];

    // Single-column foreign keys live on the column in the snapshot.
    for column in &et.columns {
        let Some((referenced_table, referenced_column)) = fk_target(column) else {
            continue;
        };
        let columns = vec![column.column_name.clone()];
        let referenced_columns = vec![referenced_column];
        match fk_status(
            live,
            &columns,
            &referenced_table,
            &referenced_columns,
            column.fk_on_delete.as_deref(),
        ) {
            FkStatus::Missing => {
                delta.changes.push(SchemaChange::AddForeignKey {
                    table: table.clone(),
                    column: column.column_name.clone(),
                    referenced_table: referenced_table.clone(),
                    referenced_column: referenced_columns[0].clone(),
                    on_delete: column.fk_on_delete.clone(),
                });
                delta.rebuild_reasons.push(format!(
                    "foreign key on column '{}' -> '{}({})'",
                    column.column_name, referenced_table, referenced_columns[0]
                ));
            }
            FkStatus::Same(position) => matched_live[position] = true,
            FkStatus::Different(position) => {
                matched_live[position] = true;
                delta.rebuild_reasons.push(format!(
                    "foreign key on column '{}' changed",
                    column.column_name
                ));
                let derived_name = MigrationEngine::foreign_key_name(
                    &table,
                    &column.column_name,
                    &referenced_table,
                );
                match live.foreign_keys[position].name.as_deref() {
                    Some(live_name) if live_name.eq_ignore_ascii_case(&derived_name) => {
                        delta.changes.push(SchemaChange::DropForeignKey {
                            table: table.clone(),
                            column: column.column_name.clone(),
                            referenced_table: referenced_table.clone(),
                        });
                        delta.changes.push(SchemaChange::AddForeignKey {
                            table: table.clone(),
                            column: column.column_name.clone(),
                            referenced_table,
                            referenced_column: referenced_columns[0].clone(),
                            on_delete: column.fk_on_delete.clone(),
                        });
                    }
                    Some(live_name) => warnings.push(DiffWarning::warning(
                        WarningKind::ManualAction,
                        Some(table.clone()),
                        format!(
                            "foreign key on '{}.{}' was created as '{}'; the generated drop statement \
                             targets '{}'. Drop and recreate the constraint manually",
                            table, column.column_name, live_name, derived_name
                        ),
                    )),
                    // No name (SQLite): the table rebuild recreates the
                    // constraint as part of the table definition.
                    None => {}
                }
            }
        }
    }

    // Composite foreign keys.
    for foreign_key in &et.foreign_keys {
        match fk_status(
            live,
            &foreign_key.columns,
            &foreign_key.referenced_table,
            &foreign_key.referenced_columns,
            foreign_key.on_delete.as_deref(),
        ) {
            FkStatus::Missing => {
                delta.changes.push(SchemaChange::AddCompositeForeignKey {
                    table: table.clone(),
                    foreign_key: foreign_key.clone(),
                });
                delta.rebuild_reasons.push(format!(
                    "foreign key '{}' ({}) -> '{}'",
                    foreign_key.name,
                    foreign_key.columns.join(", "),
                    foreign_key.referenced_table
                ));
            }
            FkStatus::Same(position) => matched_live[position] = true,
            FkStatus::Different(position) => {
                matched_live[position] = true;
                delta
                    .rebuild_reasons
                    .push(format!("foreign key '{}' changed", foreign_key.name));
                let live_fk = &live.foreign_keys[position];
                let Some(live_name) = live_fk.name.as_deref() else {
                    warnings.push(DiffWarning::warning(
                        WarningKind::ManualAction,
                        Some(table.clone()),
                        format!(
                            "composite foreign key '{}' on '{}' changed but the live constraint name \
                             is unknown; drop/recreate it manually",
                            foreign_key.name, table
                        ),
                    ));
                    continue;
                };
                let live_snapshot = SnapshotForeignKey {
                    name: live_name.to_string(),
                    columns: live_fk.columns.clone(),
                    referenced_table: live_fk.referenced_table.clone(),
                    referenced_columns: live_fk.referenced_columns.clone(),
                    on_delete: live_fk.on_delete.clone(),
                };
                delta.changes.push(SchemaChange::DropCompositeForeignKey {
                    table: table.clone(),
                    foreign_key: live_snapshot,
                });
                delta.changes.push(SchemaChange::AddCompositeForeignKey {
                    table: table.clone(),
                    foreign_key: foreign_key.clone(),
                });
            }
        }
    }

    for (position, live_fk) in live.foreign_keys.iter().enumerate() {
        if matched_live[position] {
            continue;
        }
        warnings.push(DiffWarning::info(
            WarningKind::SkippedRemoval,
            Some(table.clone()),
            format!(
                "foreign key ({}) -> '{}({})' exists in the live database but not in the model; \
                 runtime diff never drops foreign keys",
                live_fk.columns.join(", "),
                live_fk.referenced_table,
                live_fk.referenced_columns.join(", ")
            ),
        ));
    }
}

enum FkStatus {
    Missing,
    Same(usize),
    Different(usize),
}

fn fk_status(
    live: &DbTable,
    columns: &[String],
    referenced_table: &str,
    referenced_columns: &[String],
    on_delete: Option<&str>,
) -> FkStatus {
    let Some((position, live_fk)) = live.foreign_keys.iter().enumerate().find(|(_, fk)| {
        eq_ci_slices(&fk.columns, columns) && fk.referenced_table.eq_ignore_ascii_case(referenced_table)
    }) else {
        return FkStatus::Missing;
    };
    let same_columns = eq_ci_slices(&live_fk.referenced_columns, referenced_columns);
    let same_action =
        normalize_on_delete(live_fk.on_delete.as_deref()) == normalize_on_delete(on_delete);
    if same_columns && same_action {
        FkStatus::Same(position)
    } else {
        FkStatus::Different(position)
    }
}

fn normalize_on_delete(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| value != "NO ACTION" && !value.is_empty())
}

// ---------------------------------------------------------------------------
// CHECK constraints
// ---------------------------------------------------------------------------

fn compare_checks(
    delta: &mut TableDelta<'_>,
    et: &SnapshotEntityType,
    live: &DbTable,
    warnings: &mut Vec<DiffWarning>,
) {
    let table = delta.table.clone();
    let Some(live_checks) = &live.checks else {
        if !et.checks.is_empty() {
            warnings.push(DiffWarning::info(
                WarningKind::ConstraintNotIntrospected,
                Some(table.clone()),
                format!(
                    "the provider could not introspect CHECK constraints on '{}'; \
                     {} model CHECK constraint(s) cannot be verified or applied automatically",
                    table,
                    et.checks.len()
                ),
            ));
        }
        return;
    };

    let mut matched_live = vec![false; live_checks.len()];
    for check in &et.checks {
        match check_status(check, live_checks) {
            CheckStatus::Exact(position) => matched_live[position] = true,
            CheckStatus::Renamed(position) => {
                matched_live[position] = true;
                delta.changes.push(SchemaChange::DropCheck {
                    table: table.clone(),
                    check: snapshot_check_from_live(&live_checks[position]),
                });
                delta.changes.push(SchemaChange::AddCheck {
                    table: table.clone(),
                    check: check.clone(),
                });
                delta
                    .rebuild_reasons
                    .push(format!("CHECK constraint '{}' renamed", check.name));
            }
            CheckStatus::Replaced(position) => {
                matched_live[position] = true;
                delta.changes.push(SchemaChange::DropCheck {
                    table: table.clone(),
                    check: snapshot_check_from_live(&live_checks[position]),
                });
                delta.changes.push(SchemaChange::AddCheck {
                    table: table.clone(),
                    check: check.clone(),
                });
                delta
                    .rebuild_reasons
                    .push(format!("CHECK constraint '{}' redefined", check.name));
            }
            CheckStatus::Missing => {
                delta.changes.push(SchemaChange::AddCheck {
                    table: table.clone(),
                    check: check.clone(),
                });
                delta
                    .rebuild_reasons
                    .push(format!("CHECK constraint '{}'", check.name));
            }
        }
    }

    for (position, live_check) in live_checks.iter().enumerate() {
        if matched_live[position] {
            continue;
        }
        warnings.push(DiffWarning::info(
            WarningKind::SkippedRemoval,
            Some(table.clone()),
            format!(
                "CHECK constraint {} on '{}' exists in the live database but not in the model; \
                 runtime diff never drops CHECK constraints",
                live_check
                    .name
                    .as_deref()
                    .map(|name| format!("'{name}'"))
                    .unwrap_or_else(|| format!("({})", live_check.expression)),
                table
            ),
        ));
    }
}

enum CheckStatus {
    Exact(usize),
    Renamed(usize),
    Replaced(usize),
    Missing,
}

fn check_status(check: &SnapshotCheck, live_checks: &[DbCheck]) -> CheckStatus {
    let same_expression = |live_check: &DbCheck| {
        normalize_expression(&check.expression) == normalize_expression(&live_check.expression)
    };
    if let Some(position) = live_checks.iter().position(|live_check| {
        same_expression(live_check)
            && live_check
                .name
                .as_deref()
                .is_none_or(|name| name.eq_ignore_ascii_case(&check.name))
    }) {
        return CheckStatus::Exact(position);
    }
    if let Some(position) = live_checks.iter().position(|live_check| {
        normalize_expression(&check.expression) == normalize_expression(&live_check.expression)
    }) {
        return CheckStatus::Renamed(position);
    }
    if let Some(position) = live_checks.iter().position(|live_check| {
        live_check
            .name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(&check.name))
    }) {
        return CheckStatus::Replaced(position);
    }
    CheckStatus::Missing
}

fn snapshot_check_from_live(live: &DbCheck) -> SnapshotCheck {
    SnapshotCheck {
        name: live
            .name
            .clone()
            .unwrap_or_else(|| "__unnamed_check__".to_string()),
        expression: live.expression.clone(),
    }
}

fn normalize_expression(expression: &str) -> String {
    expression
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn eq_ci(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn eq_ci_slices<S: AsRef<str>>(left: &[S], right: &[S]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| eq_ci(left.as_ref(), right.as_ref()))
}
