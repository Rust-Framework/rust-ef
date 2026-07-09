//! Migration engine ?model snapshot diffing, migration generation, and history tracking.
//!
//! Corresponds to EFCore's migration system. Implements full model diffing:
//!   - Detect added/removed tables
//!   - Detect added/removed/altered columns
//!   - Generate Up/Down SQL with dialect-specific type mappings
//!   - Maintain `__ef_migrations_history` tracking table

use crate::error::EFResult;
use crate::metadata::{EntityTypeMeta, NavigationKind, NavigationMeta};
use std::collections::{HashMap, HashSet};

/// Represents a single migration with up/down SQL scripts.
#[derive(Debug, Clone)]
pub struct Migration {
    pub id: String,
    pub description: String,
    pub up_sql: String,
    pub down_sql: String,
}

/// A snapshot of the entity model at a point in time.
/// Corresponds to EFCore's `ModelSnapshot`.
#[derive(Debug, Clone)]
pub struct ModelSnapshot {
    pub migration_id: String,
    pub entity_types: Vec<SnapshotEntityType>,
}

/// Serialized form of EntityTypeMeta for snapshot storage.
#[derive(Debug, Clone)]
pub struct SnapshotEntityType {
    pub type_name: String,
    pub table_name: String,
    pub columns: Vec<SnapshotColumn>,
}

/// Serialized form of PropertyMeta for snapshot storage.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnapshotColumn {
    pub field_name: String,
    pub column_name: String,
    pub type_name: String,
    pub is_primary_key: bool,
    pub is_required: bool,
    pub is_foreign_key: bool,
    pub max_length: Option<usize>,
    pub is_auto_increment: bool,
    /// Whether this column is backed by a database sequence (PostgreSQL).
    pub is_sequence: bool,
    /// The sequence name when `is_sequence` is true.
    pub sequence_name: Option<String>,
    /// Referenced table when `is_foreign_key` is true.
    pub fk_referenced_table: Option<String>,
    /// Referenced column when `is_foreign_key` is true.
    pub fk_referenced_column: Option<String>,
    /// Non-unique index on this column.
    pub has_index: bool,
    /// Unique constraint/index on this column.
    pub is_unique: bool,
    /// ON DELETE clause for the FK constraint (e.g. "CASCADE", "SET NULL").
    /// `None` means no explicit clause (DB default applies).
    pub fk_on_delete: Option<String>,
}

/// Specifies the database SQL dialect for migration generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDialect {
    Postgres,
    MySql,
    Sqlite,
}

impl MigrationDialect {
    /// Quote an identifier according to dialect rules.
    pub fn quote(&self, ident: &str) -> String {
        match self {
            MigrationDialect::Postgres | MigrationDialect::Sqlite => format!("\"{}\"", ident),
            MigrationDialect::MySql => format!("`{}`", ident),
        }
    }

    /// Map a Rust type name to the dialect-specific column type.
    pub fn map_column_type(&self, col: &SnapshotColumn) -> String {
        // type_name comes from std::any::type_name::<T>() which returns
        // fully-qualified paths (e.g. "alloc::string::String"). Use ends_with
        // / contains matching to handle both simple and qualified names.
        let tn = col.type_name.as_str();

        // Auto-increment handling (must be checked before plain i32/i64)
        if col.is_auto_increment {
            if tn.ends_with("i32") {
                return match self {
                    MigrationDialect::Postgres => "SERIAL".into(),
                    MigrationDialect::MySql => "INT AUTO_INCREMENT".into(),
                    MigrationDialect::Sqlite => "INTEGER".into(),
                };
            }
            if tn.ends_with("i64") {
                return match self {
                    MigrationDialect::Postgres => "BIGSERIAL".into(),
                    MigrationDialect::MySql => "BIGINT AUTO_INCREMENT".into(),
                    MigrationDialect::Sqlite => "INTEGER".into(),
                };
            }
        }

        // Sequence handling (PostgreSQL: plain type + DEFAULT nextval in DDL;
        // non-PG: fall back to auto_increment syntax)
        if col.is_sequence {
            if tn.ends_with("i32") {
                return match self {
                    MigrationDialect::Postgres => "INTEGER".into(),
                    MigrationDialect::MySql => "INT AUTO_INCREMENT".into(),
                    MigrationDialect::Sqlite => "INTEGER".into(),
                };
            }
            if tn.ends_with("i64") {
                return match self {
                    MigrationDialect::Postgres => "BIGINT".into(),
                    MigrationDialect::MySql => "BIGINT AUTO_INCREMENT".into(),
                    MigrationDialect::Sqlite => "INTEGER".into(),
                };
            }
        }

        let base: &str = if tn.ends_with("i16") {
            "SMALLINT"
        } else if tn.ends_with("i32") {
            "INTEGER"
        } else if tn.ends_with("i64") {
            "BIGINT"
        } else if tn.ends_with("f32") {
            "REAL"
        } else if tn.ends_with("f64") {
            "DOUBLE PRECISION"
        } else if tn.ends_with("bool") {
            "BOOLEAN"
        } else if tn.ends_with("String") {
            return match col.max_length {
                Some(n) => format!("VARCHAR({})", n),
                None => "TEXT".into(),
            };
        } else if tn.ends_with("Vec<u8>") {
            return match self {
                MigrationDialect::Postgres => "BYTEA".into(),
                MigrationDialect::MySql | MigrationDialect::Sqlite => "BLOB".into(),
            };
        } else if tn.contains("NaiveDateTime") {
            return match self {
                MigrationDialect::Postgres => "TIMESTAMP".into(),
                MigrationDialect::MySql => "DATETIME".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("NaiveDate") {
            return match self {
                MigrationDialect::Postgres => "DATE".into(),
                MigrationDialect::MySql => "DATE".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("DateTime") {
            // chrono::DateTime<Utc> → TIMESTAMPTZ (PG) / DATETIME (MySQL) / TEXT (SQLite)
            return match self {
                MigrationDialect::Postgres => "TIMESTAMPTZ".into(),
                MigrationDialect::MySql => "DATETIME".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("Uuid") {
            return match self {
                MigrationDialect::Postgres => "UUID".into(),
                MigrationDialect::MySql => "CHAR(36)".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else if tn.contains("Decimal") {
            return match self {
                MigrationDialect::Postgres => "NUMERIC".into(),
                MigrationDialect::MySql => "DECIMAL(38,18)".into(),
                MigrationDialect::Sqlite => "TEXT".into(),
            };
        } else {
            "TEXT"
        };
        base.to_string()
    }
}

/// Single atomic schema change detected during diffing.
#[derive(Debug, Clone)]
pub(crate) enum SchemaChange {
    CreateTable {
        table: String,
        columns: Vec<SnapshotColumn>,
    },
    DropTable {
        table: String,
    },
    AddColumn {
        table: String,
        column: SnapshotColumn,
    },
    DropColumn {
        table: String,
        column_name: String,
    },
    AlterColumn {
        table: String,
        column_name: String,
        old: Box<SnapshotColumn>,
        new: Box<SnapshotColumn>,
    },
    AddForeignKey {
        table: String,
        column: String,
        referenced_table: String,
        referenced_column: String,
        on_delete: Option<String>,
    },
    DropForeignKey {
        table: String,
        column: String,
        referenced_table: String,
    },
    CreateIndex {
        table: String,
        column: String,
        is_unique: bool,
    },
    DropIndex {
        table: String,
        column: String,
        is_unique: bool,
    },
}

/// The migration engine ?compares old and new model snapshots to generate
/// migration SQL.
pub struct MigrationEngine {
    dialect: MigrationDialect,
}

impl MigrationEngine {
    pub fn new(dialect: MigrationDialect) -> Self {
        Self { dialect }
    }

    /// Generates a migration by diffing the current model against a snapshot.
    pub fn generate(
        &self,
        name: &str,
        current: &[EntityTypeMeta],
        previous_snapshot: &Option<ModelSnapshot>,
    ) -> EFResult<Migration> {
        let current_snapshot = self.create_snapshot("__current__", current);

        let changes = match previous_snapshot {
            Some(prev) => self.diff(prev, &current_snapshot),
            None => self.initial_create_with_fks(&current_snapshot),
        };

        let up_sql = self.generate_up_sql(&changes);
        let down_sql = self.generate_down_sql(&changes);

        Ok(Migration {
            id: name.to_string(),
            description: name.to_string(),
            up_sql,
            down_sql,
        })
    }

    /// Creates a snapshot from current entity type metadata.
    pub fn create_snapshot(
        &self,
        migration_id: &str,
        entity_types: &[EntityTypeMeta],
    ) -> ModelSnapshot {
        let types = entity_types
            .iter()
            .map(|et| SnapshotEntityType {
                type_name: et.type_name.to_string(),
                table_name: et.table_name.to_string(),
                columns: et
                    .properties
                    .iter()
                    .filter(|p| !p.is_not_mapped)
                    .map(|p| {
                        let (fk_table, fk_col, fk_on_delete) =
                            fk_reference_for_property(et, p.column_name.as_ref());
                        SnapshotColumn {
                            field_name: p.field_name.to_string(),
                            column_name: p.column_name.to_string(),
                            type_name: p.type_name.to_string(),
                            is_primary_key: p.is_primary_key,
                            is_required: p.is_required,
                            is_foreign_key: p.is_foreign_key,
                            max_length: p.max_length,
                            is_auto_increment: p.is_auto_increment,
                            is_sequence: p.is_sequence,
                            sequence_name: p.sequence_name.as_ref().map(|s| s.to_string()),
                            fk_referenced_table: fk_table,
                            fk_referenced_column: fk_col,
                            has_index: p.has_index,
                            is_unique: p.is_unique,
                            fk_on_delete,
                        }
                    })
                    .collect(),
            })
            .collect();

        ModelSnapshot {
            migration_id: migration_id.to_string(),
            entity_types: types,
        }
    }

    // -----------------------------------------------------------------------
    // Diffing
    // -----------------------------------------------------------------------

    fn initial_create(&self, current: &ModelSnapshot) -> Vec<SchemaChange> {
        current
            .entity_types
            .iter()
            .map(|et| SchemaChange::CreateTable {
                table: et.table_name.clone(),
                columns: et.columns.clone(),
            })
            .collect()
    }

    fn diff(&self, old: &ModelSnapshot, new: &ModelSnapshot) -> Vec<SchemaChange> {
        let mut changes = Vec::new();

        // Build lookup maps by table name
        let old_tables: HashMap<&str, &SnapshotEntityType> = old
            .entity_types
            .iter()
            .map(|e| (e.table_name.as_str(), e))
            .collect();
        let new_tables: HashMap<&str, &SnapshotEntityType> = new
            .entity_types
            .iter()
            .map(|e| (e.table_name.as_str(), e))
            .collect();

        let old_names: HashSet<&str> = old_tables.keys().copied().collect();
        let new_names: HashSet<&str> = new_tables.keys().copied().collect();

        // Dropped tables
        for name in old_names.difference(&new_names) {
            changes.push(SchemaChange::DropTable {
                table: name.to_string(),
            });
        }

        // New tables
        for name in new_names.difference(&old_names) {
            let et = new_tables[name];
            changes.push(SchemaChange::CreateTable {
                table: et.table_name.clone(),
                columns: et.columns.clone(),
            });
            Self::append_create_table_fks(&mut changes, &et.table_name, &et.columns);
            Self::append_create_table_indexes(&mut changes, &et.table_name, &et.columns);
        }

        // Tables present in both ?compare columns
        for name in old_names.intersection(&new_names) {
            let old_et = old_tables[name];
            let new_et = new_tables[name];
            let table = &old_et.table_name;

            let old_cols: HashMap<&str, &SnapshotColumn> = old_et
                .columns
                .iter()
                .map(|c| (c.column_name.as_str(), c))
                .collect();
            let new_cols: HashMap<&str, &SnapshotColumn> = new_et
                .columns
                .iter()
                .map(|c| (c.column_name.as_str(), c))
                .collect();

            let old_col_names: HashSet<&str> = old_cols.keys().copied().collect();
            let new_col_names: HashSet<&str> = new_cols.keys().copied().collect();

            // Added columns (with indexes if configured)
            for col_name in new_col_names.difference(&old_col_names) {
                let col = new_cols[col_name];
                changes.push(SchemaChange::AddColumn {
                    table: table.clone(),
                    column: (*col).clone(),
                });
                changes.extend(diff_indexes(table, &SnapshotColumn::default(), col));
            }

            // Foreign keys (after add-column, before drop-column)
            changes.extend(diff_foreign_keys(table, old_et, new_et));

            // Dropped columns
            for col_name in old_col_names.difference(&new_col_names) {
                changes.push(SchemaChange::DropColumn {
                    table: table.clone(),
                    column_name: col_name.to_string(),
                });
            }

            // Altered columns and index changes (present in both)
            for col_name in old_col_names.intersection(&new_col_names) {
                let old_col = old_cols[col_name];
                let new_col = new_cols[col_name];

                // Index state changes → CreateIndex/DropIndex
                changes.extend(diff_indexes(table, old_col, new_col));

                // Structural changes (excluding index fields) → AlterColumn
                if !columns_structurally_equal(old_col, new_col) {
                    changes.push(SchemaChange::AlterColumn {
                        table: table.clone(),
                        column_name: col_name.to_string(),
                        old: Box::new((*old_col).clone()),
                        new: Box::new((*new_col).clone()),
                    });
                }
            }
        }

        changes
    }

    fn append_create_table_fks(
        changes: &mut Vec<SchemaChange>,
        table: &str,
        columns: &[SnapshotColumn],
    ) {
        for col in columns {
            if let Some((ref_table, ref_col)) = fk_target(col) {
                changes.push(SchemaChange::AddForeignKey {
                    table: table.to_string(),
                    column: col.column_name.clone(),
                    referenced_table: ref_table,
                    referenced_column: ref_col,
                    on_delete: col.fk_on_delete.clone(),
                });
            }
        }
    }

    /// Emits `CreateIndex` changes for indexed columns in a newly created table.
    fn append_create_table_indexes(
        changes: &mut Vec<SchemaChange>,
        table: &str,
        columns: &[SnapshotColumn],
    ) {
        for col in columns {
            if col.is_unique {
                changes.push(SchemaChange::CreateIndex {
                    table: table.to_string(),
                    column: col.column_name.clone(),
                    is_unique: true,
                });
            } else if col.has_index {
                changes.push(SchemaChange::CreateIndex {
                    table: table.to_string(),
                    column: col.column_name.clone(),
                    is_unique: false,
                });
            }
        }
    }
}

fn fk_target(col: &SnapshotColumn) -> Option<(String, String)> {
    if !col.is_foreign_key {
        return None;
    }
    Some((
        col.fk_referenced_table.clone()?,
        col.fk_referenced_column.clone()?,
    ))
}

/// Standard index name used by CreateIndex/DropIndex SQL generation.
fn index_name(table: &str, column: &str) -> String {
    format!("ix_{}_{}", table, column)
}

fn fk_reference_for_property(
    meta: &EntityTypeMeta,
    column_name: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    for nav in &meta.navigations {
        if nav.kind != NavigationKind::BelongsTo {
            continue;
        }
        let matches = nav
            .fk_column
            .as_ref()
            .is_some_and(|fk| fk.as_ref() == column_name);
        if matches {
            let on_delete = resolve_fk_on_delete_clause(nav, meta);
            return (
                nav.related_table.as_ref().map(|s| s.to_string()),
                nav.referenced_key_column.as_ref().map(|s| s.to_string()),
                on_delete,
            );
        }
    }
    (None, None, None)
}

/// Resolves the `ON DELETE` SQL clause for a FK by checking:
/// 1. The child's BelongsTo navigation `delete_behavior` (if user configured it here)
/// 2. The principal's inverse HasMany navigation via `resolve_delete_behavior`
/// 3. Fallback: nullability of the FK property on the child entity
fn resolve_fk_on_delete_clause(
    belongs_to_nav: &NavigationMeta,
    child_meta: &EntityTypeMeta,
) -> Option<String> {
    use crate::relations::DeleteBehavior;

    // 1. Explicit on the BelongsTo side
    if let Some(b) = belongs_to_nav.delete_behavior {
        return Some(b.to_sql_clause().to_string());
    }

    // 2. Find the principal's HasMany navigation pointing back
    if let Some(meta_fn) = belongs_to_nav.related_entity_meta {
        let principal_meta = meta_fn();
        for principal_nav in &principal_meta.navigations {
            if principal_nav.kind == NavigationKind::HasMany
                && principal_nav.related_type_id == child_meta.type_id
            {
                let behavior = crate::db_context::resolve_delete_behavior(principal_nav);
                return Some(behavior.to_sql_clause().to_string());
            }
        }
    }

    // 3. Fallback: nullability from the FK property's Rust type
    if let Some(fk_prop) = child_meta.properties.iter().find(|p| p.is_foreign_key) {
        let is_nullable = fk_prop.type_name.contains("Option");
        let behavior = if is_nullable {
            DeleteBehavior::Restrict
        } else {
            DeleteBehavior::Cascade
        };
        return Some(behavior.to_sql_clause().to_string());
    }

    None
}

fn diff_foreign_keys(
    table: &str,
    old_et: &SnapshotEntityType,
    new_et: &SnapshotEntityType,
) -> Vec<SchemaChange> {
    let mut changes = Vec::new();
    let old_fks: HashMap<&str, (&SnapshotColumn, String, String, Option<String>)> = old_et
        .columns
        .iter()
        .filter_map(|c| {
            let (rt, rc) = fk_target(c)?;
            Some((c.column_name.as_str(), (c, rt, rc, c.fk_on_delete.clone())))
        })
        .collect();
    let new_fks: HashMap<&str, (&SnapshotColumn, String, String, Option<String>)> = new_et
        .columns
        .iter()
        .filter_map(|c| {
            let (rt, rc) = fk_target(c)?;
            Some((c.column_name.as_str(), (c, rt, rc, c.fk_on_delete.clone())))
        })
        .collect();

    let old_names: HashSet<&str> = old_fks.keys().copied().collect();
    let new_names: HashSet<&str> = new_fks.keys().copied().collect();

    for col in new_names.difference(&old_names) {
        let (_, rt, rc, od) = &new_fks[col];
        changes.push(SchemaChange::AddForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: rt.clone(),
            referenced_column: rc.clone(),
            on_delete: od.clone(),
        });
    }

    for col in old_names.difference(&new_names) {
        let (_, rt, _, _) = &old_fks[col];
        changes.push(SchemaChange::DropForeignKey {
            table: table.to_string(),
            column: (*col).to_string(),
            referenced_table: rt.clone(),
        });
    }

    for col in old_names.intersection(&new_names) {
        let (_, old_rt, old_rc, old_od) = &old_fks[col];
        let (_, new_rt, new_rc, new_od) = &new_fks[col];
        if old_rt != new_rt || old_rc != new_rc || old_od != new_od {
            changes.push(SchemaChange::DropForeignKey {
                table: table.to_string(),
                column: (*col).to_string(),
                referenced_table: old_rt.clone(),
            });
            changes.push(SchemaChange::AddForeignKey {
                table: table.to_string(),
                column: (*col).to_string(),
                referenced_table: new_rt.clone(),
                referenced_column: new_rc.clone(),
                on_delete: new_od.clone(),
            });
        }
    }

    changes
}

/// Compares two columns for structural equality, ignoring index state
/// (`has_index` / `is_unique`). Index changes are handled separately by
/// `diff_indexes` so they emit `CreateIndex`/`DropIndex` instead of
/// `AlterColumn`.
fn columns_structurally_equal(a: &SnapshotColumn, b: &SnapshotColumn) -> bool {
    a.field_name == b.field_name
        && a.column_name == b.column_name
        && a.type_name == b.type_name
        && a.is_primary_key == b.is_primary_key
        && a.is_required == b.is_required
        && a.is_foreign_key == b.is_foreign_key
        && a.max_length == b.max_length
        && a.is_auto_increment == b.is_auto_increment
        && a.fk_referenced_table == b.fk_referenced_table
        && a.fk_referenced_column == b.fk_referenced_column
}

/// Returns `CreateIndex`/`DropIndex` changes when a column's index state
/// transitions between None / NonUnique / Unique.
fn diff_indexes(table: &str, old: &SnapshotColumn, new: &SnapshotColumn) -> Vec<SchemaChange> {
    let old_kind = index_kind(old);
    let new_kind = index_kind(new);
    if old_kind == new_kind {
        return Vec::new();
    }
    let mut changes = Vec::new();
    if old_kind != IndexKind::None {
        changes.push(SchemaChange::DropIndex {
            table: table.to_string(),
            column: new.column_name.clone(),
            is_unique: old_kind == IndexKind::Unique,
        });
    }
    if new_kind != IndexKind::None {
        changes.push(SchemaChange::CreateIndex {
            table: table.to_string(),
            column: new.column_name.clone(),
            is_unique: new_kind == IndexKind::Unique,
        });
    }
    changes
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IndexKind {
    None,
    NonUnique,
    Unique,
}

fn index_kind(col: &SnapshotColumn) -> IndexKind {
    if col.is_unique {
        IndexKind::Unique
    } else if col.has_index {
        IndexKind::NonUnique
    } else {
        IndexKind::None
    }
}

impl MigrationEngine {
    fn initial_create_with_fks(&self, current: &ModelSnapshot) -> Vec<SchemaChange> {
        let mut changes = self.initial_create(current);
        for et in &current.entity_types {
            Self::append_create_table_fks(&mut changes, &et.table_name, &et.columns);
            Self::append_create_table_indexes(&mut changes, &et.table_name, &et.columns);
        }
        changes
    }

    // -----------------------------------------------------------------------
    // SQL generation
    // -----------------------------------------------------------------------

    /// Standard foreign-key constraint name used by Add/DropForeignKey.
    pub fn foreign_key_name(table: &str, column: &str, referenced_table: &str) -> String {
        format!("fk_{}_{}_{}", table, column, referenced_table)
    }

    /// Standard index name used by CreateIndex/DropIndex.
    pub fn index_name(table: &str, column: &str) -> String {
        format!("ix_{}_{}", table, column)
    }

    pub fn generate_alter_column_sql(
        &self,
        table: &str,
        column_name: &str,
        new: &SnapshotColumn,
    ) -> String {
        let q = |s: &str| self.dialect.quote(s);
        let col_type = self.dialect.map_column_type(new);
        match self.dialect {
            MigrationDialect::Postgres => {
                let nullable = if new.is_required {
                    "SET NOT NULL"
                } else {
                    "DROP NOT NULL"
                };
                format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {};\nALTER TABLE {} ALTER COLUMN {} {};\n",
                    q(table),
                    q(column_name),
                    col_type,
                    q(table),
                    q(column_name),
                    nullable
                )
            }
            MigrationDialect::MySql => {
                let nullable = if new.is_required { "NOT NULL" } else { "NULL" };
                format!(
                    "ALTER TABLE {} MODIFY COLUMN {} {} {};\n",
                    q(table),
                    q(column_name),
                    col_type,
                    nullable
                )
            }
            MigrationDialect::Sqlite => format!(
                "-- SQLite does not support ALTER COLUMN type changes; \
                 rebuild table manually for {}.{}\n",
                q(table),
                q(column_name)
            ),
        }
    }

    fn generate_up_sql(&self, changes: &[SchemaChange]) -> String {
        self.generate_up_sql_inner(changes, true)
    }

    fn generate_ddl_sql(&self, changes: &[SchemaChange]) -> String {
        self.generate_up_sql_inner(changes, false)
    }

    fn generate_up_sql_inner(&self, changes: &[SchemaChange], record_history: bool) -> String {
        let mut sql = String::from("-- Up Migration (auto-generated by rust-ef)\n\n");
        let q = |s: &str| self.dialect.quote(s);
        let create_kw = if record_history {
            "CREATE TABLE"
        } else {
            "CREATE TABLE IF NOT EXISTS"
        };

        for change in changes {
            match change {
                SchemaChange::CreateTable { table, columns } => {
                    // PostgreSQL: create sequences before the table
                    if self.dialect == MigrationDialect::Postgres {
                        for c in columns {
                            if c.is_sequence {
                                if let Some(seq_name) = &c.sequence_name {
                                    sql.push_str(&format!(
                                        "CREATE SEQUENCE IF NOT EXISTS {};\n",
                                        q(seq_name)
                                    ));
                                }
                            }
                        }
                    }

                    sql.push_str(&format!("{} {} (\n", create_kw, q(table)));

                    // Separate primary key columns from regular columns
                    let pk_columns: Vec<&str> = columns
                        .iter()
                        .filter(|c| c.is_primary_key)
                        .map(|c| c.column_name.as_str())
                        .collect();

                    let col_defs: Vec<String> = columns
                        .iter()
                        .map(|c| {
                            let nullable = if c.is_required { "NOT NULL" } else { "NULL" };
                            let col_type = self.dialect.map_column_type(c);
                            // PostgreSQL sequence column: add DEFAULT nextval('seq_name')
                            if c.is_sequence && self.dialect == MigrationDialect::Postgres {
                                if let Some(seq_name) = &c.sequence_name {
                                    return format!(
                                        "{} {} DEFAULT nextval('{}') {}",
                                        q(&c.column_name),
                                        col_type,
                                        seq_name,
                                        nullable
                                    );
                                }
                            }
                            // Don't put PRIMARY KEY on individual columns; handle separately
                            [q(&c.column_name), col_type, nullable.to_string()]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .collect();
                    sql.push_str(&format!("    {}\n", col_defs.join(",\n    ")));

                    // Composite primary key constraint
                    if pk_columns.len() == 1 {
                        sql.push_str(&format!("    ,PRIMARY KEY ({})\n", q(pk_columns[0])));
                    } else if pk_columns.len() > 1 {
                        let pk_list: Vec<String> = pk_columns.iter().map(|c| q(c)).collect();
                        sql.push_str(&format!("    ,PRIMARY KEY ({})\n", pk_list.join(", ")));
                    }

                    sql.push_str(");\n\n");
                }
                SchemaChange::DropTable { table } => {
                    sql.push_str(&format!("DROP TABLE IF EXISTS {};\n", q(table)));
                }
                SchemaChange::AddColumn { table, column } => {
                    let col_type = self.dialect.map_column_type(column);
                    let nullable = if column.is_required {
                        "NOT NULL"
                    } else {
                        "NULL"
                    };
                    sql.push_str(&format!(
                        "ALTER TABLE {} ADD COLUMN {} {} {};\n",
                        q(table),
                        q(&column.column_name),
                        col_type,
                        nullable
                    ));
                }
                SchemaChange::DropColumn { table, column_name } => {
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP COLUMN IF EXISTS {};\n",
                        q(table),
                        q(column_name)
                    ));
                }
                SchemaChange::AlterColumn {
                    table,
                    column_name,
                    old: _,
                    new,
                } => {
                    sql.push_str(&self.generate_alter_column_sql(table, column_name, new));
                }
                SchemaChange::AddForeignKey {
                    table,
                    column,
                    referenced_table,
                    referenced_column,
                    on_delete,
                } => {
                    let fk_name = Self::foreign_key_name(table, column, referenced_table);
                    let on_delete_clause = on_delete
                        .as_deref()
                        .map(|c| format!(" ON DELETE {}", c))
                        .unwrap_or_default();
                    sql.push_str(&format!(
                        "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}){};\n",
                        q(table),
                        q(&fk_name),
                        q(column),
                        q(referenced_table),
                        q(referenced_column),
                        on_delete_clause
                    ));
                }
                SchemaChange::DropForeignKey {
                    table,
                    column,
                    referenced_table,
                } => {
                    let fk_name = Self::foreign_key_name(table, column, referenced_table);
                    match self.dialect {
                        MigrationDialect::MySql => sql.push_str(&format!(
                            "ALTER TABLE {} DROP FOREIGN KEY {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                        _ => sql.push_str(&format!(
                            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                    }
                }
                SchemaChange::CreateIndex {
                    table,
                    column,
                    is_unique,
                } => {
                    let unique_kw = if *is_unique { "UNIQUE " } else { "" };
                    let idx_name = index_name(table, column);
                    sql.push_str(&format!(
                        "CREATE {unique_kw}INDEX {} ON {} ({});\n",
                        q(&idx_name),
                        q(table),
                        q(column)
                    ));
                }
                SchemaChange::DropIndex {
                    table,
                    column,
                    is_unique: _,
                } => {
                    let idx_name = index_name(table, column);
                    match self.dialect {
                        MigrationDialect::MySql => {
                            sql.push_str(&format!("DROP INDEX {} ON {};\n", q(&idx_name), q(table)))
                        }
                        _ => sql.push_str(&format!("DROP INDEX IF EXISTS {};\n", q(&idx_name))),
                    }
                }
            }
        }

        if record_history {
            sql.push_str(&format!(
                "INSERT INTO {}(migration_id, product_version) VALUES ('{{migration_id}}', '{}');\n",
                q(MIGRATION_HISTORY_TABLE),
                PRODUCT_VERSION
            ));
        }

        sql
    }

    fn generate_down_sql(&self, changes: &[SchemaChange]) -> String {
        let mut sql = String::from("-- Down Migration (auto-generated by rust-ef)\n\n");
        let q = |s: &str| self.dialect.quote(s);

        // Reverse the changes for the down migration
        for change in changes.iter().rev() {
            match change {
                SchemaChange::CreateTable { table, .. } => {
                    sql.push_str(&format!("DROP TABLE IF EXISTS {};\n", q(table)));
                }
                SchemaChange::DropTable { table } => {
                    // Can't fully restore a dropped table ?log warning
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore table {} (original schema unknown)\n",
                        q(table)
                    ));
                }
                SchemaChange::AddColumn { table, column } => {
                    sql.push_str(&format!(
                        "ALTER TABLE {} DROP COLUMN IF EXISTS {};\n",
                        q(table),
                        q(&column.column_name)
                    ));
                }
                SchemaChange::DropColumn { table, column_name } => {
                    // Can't fully restore dropped column without original type
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore column {} on {} (original type unknown)\n",
                        q(column_name),
                        q(table)
                    ));
                }
                SchemaChange::AlterColumn {
                    table,
                    column_name,
                    old,
                    new: _,
                } => {
                    sql.push_str(&self.generate_alter_column_sql(table, column_name, old));
                }
                SchemaChange::AddForeignKey {
                    table,
                    column,
                    referenced_table,
                    ..
                } => {
                    let fk_name = Self::foreign_key_name(table, column, referenced_table);
                    match self.dialect {
                        MigrationDialect::MySql => sql.push_str(&format!(
                            "ALTER TABLE {} DROP FOREIGN KEY {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                        _ => sql.push_str(&format!(
                            "ALTER TABLE {} DROP CONSTRAINT IF EXISTS {};\n",
                            q(table),
                            q(&fk_name)
                        )),
                    }
                }
                SchemaChange::DropForeignKey {
                    table,
                    column,
                    referenced_table,
                    ..
                } => {
                    sql.push_str(&format!(
                        "-- WARNING: Cannot restore foreign key constraint {} on {}.{}\n",
                        q(&Self::foreign_key_name(table, column, referenced_table)),
                        q(table),
                        q(column)
                    ));
                }
                SchemaChange::CreateIndex {
                    table,
                    column,
                    is_unique: _,
                } => {
                    let idx_name = index_name(table, column);
                    match self.dialect {
                        MigrationDialect::MySql => {
                            sql.push_str(&format!("DROP INDEX {} ON {};\n", q(&idx_name), q(table)))
                        }
                        _ => sql.push_str(&format!("DROP INDEX IF EXISTS {};\n", q(&idx_name))),
                    }
                }
                SchemaChange::DropIndex {
                    table,
                    column,
                    is_unique,
                } => {
                    let unique_kw = if *is_unique { "UNIQUE " } else { "" };
                    let idx_name = index_name(table, column);
                    sql.push_str(&format!(
                        "CREATE {unique_kw}INDEX {} ON {} ({});\n",
                        q(&idx_name),
                        q(table),
                        q(column)
                    ));
                }
            }
        }

        sql.push_str(&format!(
            "DELETE FROM {} WHERE migration_id = '{{migration_id}}';\n",
            q(MIGRATION_HISTORY_TABLE)
        ));

        sql
    }

    /// Ensures the migration history table exists.
    pub async fn ensure_history_table(
        &self,
        provider: &dyn crate::provider::IDatabaseProvider,
    ) -> EFResult<()> {
        let sql = create_migration_history_table_sql(self.dialect);
        provider.execute_migration_command(&sql).await
    }

    /// Applies a migration's Up SQL against a database provider.
    pub async fn apply(
        &self,
        provider: &dyn crate::provider::IDatabaseProvider,
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
        provider: &dyn crate::provider::IDatabaseProvider,
        migration: &Migration,
    ) -> EFResult<()> {
        if !self.is_applied(provider, &migration.id).await? {
            return Err(crate::error::EFError::migration(format!(
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
        provider: &dyn crate::provider::IDatabaseProvider,
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
        provider: &dyn crate::provider::IDatabaseProvider,
        migration_id: &str,
    ) -> EFResult<bool> {
        let applied = self.get_applied_migrations(provider).await?;
        Ok(applied.iter().any(|e| e.migration_id == migration_id))
    }

    /// Applies all pending migrations in order, skipping already-applied ids.
    pub async fn apply_pending(
        &self,
        provider: &dyn crate::provider::IDatabaseProvider,
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
        provider: &dyn crate::provider::IDatabaseProvider,
        migrations: &[Migration],
    ) -> EFResult<Option<String>> {
        let applied = self.get_applied_migrations(provider).await?;
        let last = applied.last().map(|e| e.migration_id.clone());
        let Some(last_id) = last else {
            return Ok(None);
        };
        let migration = migrations.iter().find(|m| m.id == last_id).ok_or_else(|| {
            crate::error::EFError::migration(format!(
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
        provider: &dyn crate::provider::IDatabaseProvider,
        migrations: &[Migration],
        target: Option<&str>,
    ) -> EFResult<Vec<String>> {
        let applied = self.get_applied_migrations(provider).await?;
        let applied_ids: Vec<&str> = applied.iter().map(|e| e.migration_id.as_str()).collect();

        let to_revert: Vec<String> = match target {
            Some(t) => {
                let idx = applied_ids.iter().position(|id| *id == t).ok_or_else(|| {
                    crate::error::EFError::migration(format!(
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
                crate::error::EFError::migration(format!(
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
                crate::error::EFError::migration(format!(
                    "from migration '{f}' not found in local set"
                ))
            })? as i64,
            None => -1,
        };
        let to_idx: i64 = match to {
            Some(t) => migrations.iter().position(|m| m.id == t).ok_or_else(|| {
                crate::error::EFError::migration(format!(
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
        provider: &dyn crate::provider::IDatabaseProvider,
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
        provider: &dyn crate::provider::IDatabaseProvider,
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
        provider: &dyn crate::provider::IDatabaseProvider,
        meta: &EntityTypeMeta,
        rows: &[std::collections::HashMap<String, crate::provider::DbValue>],
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

/// Represents a record in the migration history table.
/// Table name: `__ef_migrations_history`
/// Corresponds to EFCore's `__EFMigrationsHistory`.
#[derive(Debug, Clone)]
pub struct MigrationHistoryEntry {
    pub migration_id: String,
    pub product_version: String,
}

/// Table name for migration history.
pub const MIGRATION_HISTORY_TABLE: &str = "__ef_migrations_history";

/// Product version recorded in migration history.
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn seed_insert_sql(
    dialect: MigrationDialect,
    table: &str,
    columns: &[&str],
    gen: &dyn crate::provider::ISqlGenerator,
) -> String {
    let quoted_table = dialect.quote(table);
    let quoted_cols: Vec<String> = columns.iter().map(|c| dialect.quote(c)).collect();
    let placeholders: Vec<String> = (0..columns.len())
        .map(|i| gen.parameter_placeholder(i + 1))
        .collect();
    let cols = quoted_cols.join(", ");
    let vals = placeholders.join(", ");
    match dialect {
        MigrationDialect::Sqlite => {
            format!("INSERT OR IGNORE INTO {quoted_table} ({cols}) VALUES ({vals})")
        }
        MigrationDialect::Postgres => {
            format!("INSERT INTO {quoted_table} ({cols}) VALUES ({vals}) ON CONFLICT DO NOTHING")
        }
        MigrationDialect::MySql => {
            format!("INSERT IGNORE INTO {quoted_table} ({cols}) VALUES ({vals})")
        }
    }
}

/// Split a migration script into individual executable statements.
fn split_sql_statements(sql: &str) -> Vec<String> {
    sql.lines()
        .filter(|l| {
            let trimmed = l.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// SQL to create the migration history table.
pub fn create_migration_history_table_sql(dialect: MigrationDialect) -> String {
    match dialect {
        MigrationDialect::Postgres => format!(
            r#"CREATE TABLE IF NOT EXISTS "{table}" (
    "migration_id" VARCHAR(150) NOT NULL PRIMARY KEY,
    "product_version" VARCHAR(32) NOT NULL,
    "applied_at" TIMESTAMPTZ NOT NULL DEFAULT NOW()
);"#,
            table = MIGRATION_HISTORY_TABLE
        ),
        MigrationDialect::MySql => format!(
            r#"CREATE TABLE IF NOT EXISTS `{table}` (
    `migration_id` VARCHAR(150) NOT NULL PRIMARY KEY,
    `product_version` VARCHAR(32) NOT NULL,
    `applied_at` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;"#,
            table = MIGRATION_HISTORY_TABLE
        ),
        MigrationDialect::Sqlite => format!(
            r#"CREATE TABLE IF NOT EXISTS "{table}" (
    "migration_id" TEXT NOT NULL PRIMARY KEY,
    "product_version" TEXT NOT NULL
);"#,
            table = MIGRATION_HISTORY_TABLE
        ),
    }
}

// ---------------------------------------------------------------------------
// Filesystem migration store (CLI / project migrations folder)
// ---------------------------------------------------------------------------

use std::fs;
use std::path::{Path, PathBuf};

/// Reads and writes migration scripts on disk.
///
/// Layout:
/// ```text
/// Migrations/
///   20260625_InitialCreate/
///     up.sql
///     down.sql
/// ```
#[derive(Debug, Clone)]
pub struct MigrationStore {
    root: PathBuf,
}

impl MigrationStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Saves a migration as `{id}/up.sql` and `{id}/down.sql`.
    pub fn save(&self, migration: &Migration) -> EFResult<()> {
        fs::create_dir_all(&self.root).map_err(migration_io_err)?;
        let dir = self.root.join(&migration.id);
        fs::create_dir_all(&dir).map_err(migration_io_err)?;
        fs::write(dir.join("up.sql"), &migration.up_sql).map_err(migration_io_err)?;
        fs::write(dir.join("down.sql"), &migration.down_sql).map_err(migration_io_err)?;
        Ok(())
    }

    /// Loads all migrations sorted by id (timestamp prefix recommended).
    pub fn load_all(&self) -> EFResult<Vec<Migration>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut ids: Vec<String> = fs::read_dir(&self.root)
            .map_err(migration_io_err)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort();
        ids.into_iter().map(|id| self.load(&id)).collect()
    }

    pub fn load(&self, id: &str) -> EFResult<Migration> {
        let dir = self.root.join(id);
        let up_sql = fs::read_to_string(dir.join("up.sql")).map_err(migration_io_err)?;
        let down_sql = fs::read_to_string(dir.join("down.sql")).map_err(migration_io_err)?;
        Ok(Migration {
            id: id.to_string(),
            description: id.to_string(),
            up_sql,
            down_sql,
        })
    }

    /// Writes a model snapshot JSON file for the next diff baseline.
    pub fn save_snapshot(&self, snapshot: &ModelSnapshot) -> EFResult<()> {
        fs::create_dir_all(&self.root).map_err(migration_io_err)?;
        let json = snapshot_to_json(snapshot);
        fs::write(self.root.join("model_snapshot.json"), json).map_err(migration_io_err)
    }

    pub fn load_snapshot(&self) -> EFResult<Option<ModelSnapshot>> {
        let path = self.root.join("model_snapshot.json");
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(path).map_err(migration_io_err)?;
        parse_model_snapshot_json(&text)
    }
}

/// Parses a model snapshot JSON file (same format as [`MigrationStore::save_snapshot`]).
pub fn parse_model_snapshot_json(text: &str) -> EFResult<Option<ModelSnapshot>> {
    snapshot_from_json(text)
}

fn migration_io_err(e: std::io::Error) -> crate::error::EFError {
    crate::error::EFError::migration(e.to_string())
}

fn snapshot_to_json(snapshot: &ModelSnapshot) -> String {
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
