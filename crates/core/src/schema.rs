//! Provider-neutral database schema description.
//!
//! This module is the single data model shared by two consumers:
//!
//! 1. **Database introspection** (provider side): `IDatabaseProvider::introspect`
//!    reads the live schema of a database and returns [`DbTable`] values.
//! 2. **Runtime model diffing** (migration side): `MigrationEngine::diff`
//!    compares a model snapshot against a `&[DbTable]` and generates the DDL
//!    needed to bring the live database in line with the model.
//!
//! Keeping the types provider-neutral (no SQL dialect, no migration concepts)
//! lets providers depend on this module without pulling in migration semantics.
//!
//! # "Unknown" vs "empty"
//!
//! Some constraint kinds cannot be introspected by every provider without
//! parsing free-form DDL (`CHECK` constraints) or are not part of the portable
//! metadata surface at all (`TRIGGER` bodies). Those fields are therefore
//! `Option`:
//!
//! - `Some(vec![])` — the provider inspected the database and there are none.
//! - `Some(vec![..])` — the provider inspected the database and found them.
//! - `None` — the provider cannot report this kind.
//!
//! The distinction matters for diffing: treating `None` as "none" would make a
//! runtime diff report a `CHECK` constraint as already applied (or add it
//! again). Callers must keep `None` when introspected data is incomplete.

/// A column as reported by live-schema introspection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbColumn {
    /// Column name.
    pub name: String,
    /// Database type as reported by the provider, e.g. `INTEGER`, `TEXT`,
    /// `VARCHAR(64)` (SQLite) or `character varying` (PostgreSQL). The diff
    /// normalizes this before comparing it with the model's mapped type.
    pub data_type: String,
    /// Whether the column is nullable according to the database.
    pub is_nullable: bool,
    /// Primary-key member.
    pub is_primary_key: bool,
    /// Auto-generated value (`AUTO_INCREMENT` / `SERIAL` / `INTEGER PRIMARY KEY`).
    pub is_auto_increment: bool,
    /// Declared maximum length for character types, when reported separately
    /// from [`data_type`](Self::data_type).
    pub max_length: Option<usize>,
    /// Raw `DEFAULT` expression as reported by the database, if any.
    pub default_value: Option<String>,
}

impl DbColumn {
    /// Whether the column rejects `NULL` values.
    ///
    /// Primary-key columns are non-nullable in every supported database even
    /// when introspection reports `is_nullable = true` (SQLite's
    /// `INTEGER PRIMARY KEY` is the common case: `PRAGMA table_info` reports
    /// `notnull = 0` for it).
    pub fn is_not_null(&self) -> bool {
        !self.is_nullable || self.is_primary_key
    }
}

/// One key part of a live index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbIndexColumn {
    /// Column name or raw SQL expression.
    pub value: String,
    /// Whether [`value`](Self::value) is a raw expression rather than a column.
    pub is_expression: bool,
}

/// An index as reported by live-schema introspection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbIndex {
    /// Index name.
    pub name: String,
    /// Key parts, in order.
    pub columns: Vec<DbIndexColumn>,
    /// `UNIQUE` index.
    pub is_unique: bool,
    /// Rendered partial-index predicate (SQLite/PostgreSQL `WHERE` clause).
    pub filter: Option<String>,
}

impl DbIndex {
    /// Returns the plain column names of this index (expression parts skipped).
    pub fn column_names(&self) -> Vec<&str> {
        self.columns
            .iter()
            .filter(|c| !c.is_expression)
            .map(|c| c.value.as_str())
            .collect()
    }
}

/// A foreign key as reported by live-schema introspection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbForeignKey {
    /// Constraint name. `None` when the provider cannot report it (SQLite keeps
    /// foreign keys unnamed in `PRAGMA foreign_key_list`; the name is only
    /// recoverable by parsing the table DDL).
    pub name: Option<String>,
    /// Dependent columns.
    pub columns: Vec<String>,
    /// Principal table.
    pub referenced_table: String,
    /// Principal columns.
    pub referenced_columns: Vec<String>,
    /// `ON DELETE` action (e.g. `CASCADE`), if any.
    pub on_delete: Option<String>,
}

/// A `CHECK` constraint as reported by live-schema introspection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbCheck {
    /// Constraint name, when the database exposes one.
    pub name: Option<String>,
    /// The checked expression, without the enclosing `CHECK (...)`.
    pub expression: String,
}

/// A `TRIGGER` as reported by live-schema introspection.
///
/// The full statement is kept verbatim so a table rebuild can recreate it after
/// the original table (and therefore the trigger) has been dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbTrigger {
    /// Trigger name.
    pub name: String,
    /// Original `CREATE TRIGGER ...` statement.
    pub sql: String,
}

/// A table as reported by live-schema introspection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DbTable {
    /// Table name.
    pub name: String,
    /// Columns in definition order.
    pub columns: Vec<DbColumn>,
    /// Indexes (including unique indexes; excluding implicit primary-key
    /// indexes when the provider can tell them apart).
    pub indexes: Vec<DbIndex>,
    /// Foreign keys.
    pub foreign_keys: Vec<DbForeignKey>,
    /// `CHECK` constraints; `None` when the provider cannot report them.
    pub checks: Option<Vec<DbCheck>>,
    /// Triggers attached to the table; `None` when the provider cannot report
    /// them. A table rebuild preserves the triggers from this list.
    pub triggers: Option<Vec<DbTrigger>>,
}

impl DbTable {
    /// Creates an empty table description with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// Looks up a column by name.
    pub fn column(&self, name: &str) -> Option<&DbColumn> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Looks up an index by name.
    pub fn index(&self, name: &str) -> Option<&DbIndex> {
        self.indexes.iter().find(|i| i.name == name)
    }

    /// Looks up a foreign key by its dependent columns (and principal table).
    ///
    /// Structural matching is used because SQLite does not expose constraint
    /// names through `PRAGMA foreign_key_list`.
    pub fn foreign_key(&self, columns: &[String], referenced_table: &str) -> Option<&DbForeignKey> {
        self.foreign_keys
            .iter()
            .find(|fk| fk.columns == columns && fk.referenced_table == referenced_table)
    }

    /// Looks up a `CHECK` constraint by name, if checks were introspected.
    pub fn check(&self, name: &str) -> Option<&DbCheck> {
        self.checks
            .as_deref()?
            .iter()
            .find(|c| c.name.as_deref() == Some(name))
    }

    /// Whether the table has a `TRIGGER` with the given name and the provider
    /// reported trigger metadata.
    pub fn trigger(&self, name: &str) -> Option<&DbTrigger> {
        self.triggers
            .as_deref()?
            .iter()
            .find(|t| t.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_table_reports_constraints_as_unknown() {
        // A provider that does not introspect CHECK constraints / triggers must
        // leave them as `None` (unknown), never as an empty `Vec` (known none).
        let table = DbTable::new("orders");
        assert_eq!(table.name, "orders");
        assert!(table.checks.is_none());
        assert!(table.triggers.is_none());
        assert!(table.columns.is_empty());
    }

    #[test]
    fn primary_key_columns_are_not_null_even_when_introspection_says_nullable() {
        // SQLite reports `INTEGER PRIMARY KEY` as nullable in `PRAGMA table_info`.
        let column = DbColumn {
            name: "id".into(),
            data_type: "INTEGER".into(),
            is_nullable: true,
            is_primary_key: true,
            is_auto_increment: true,
            ..Default::default()
        };
        assert!(column.is_not_null());

        let nullable = DbColumn {
            name: "note".into(),
            is_nullable: true,
            ..Default::default()
        };
        assert!(!nullable.is_not_null());
    }

    #[test]
    fn table_lookup_helpers_match_by_name_and_shape() {
        let table = DbTable {
            name: "posts".into(),
            columns: vec![DbColumn {
                name: "blog_id".into(),
                ..Default::default()
            }],
            indexes: vec![DbIndex {
                name: "ix_posts_blog_id".into(),
                columns: vec![DbIndexColumn {
                    value: "blog_id".into(),
                    is_expression: false,
                }],
                ..Default::default()
            }],
            foreign_keys: vec![DbForeignKey {
                columns: vec!["blog_id".into()],
                referenced_table: "blogs".into(),
                referenced_columns: vec!["id".into()],
                ..Default::default()
            }],
            checks: Some(vec![DbCheck {
                name: Some("ck_posts_title".into()),
                expression: "length(title) > 0".into(),
            }]),
            triggers: Some(vec![DbTrigger {
                name: "trg_posts".into(),
                sql: "CREATE TRIGGER trg_posts AFTER INSERT ON posts BEGIN SELECT 1; END".into(),
            }]),
        };

        assert!(table.column("blog_id").is_some());
        assert!(table.column("missing").is_none());
        assert!(table.index("ix_posts_blog_id").is_some());
        assert_eq!(
            table.index("ix_posts_blog_id").unwrap().column_names(),
            vec!["blog_id"]
        );
        assert!(table
            .foreign_key(&["blog_id".to_string()], "blogs")
            .is_some());
        assert!(table
            .foreign_key(&["blog_id".to_string()], "users")
            .is_none());
        assert!(table.check("ck_posts_title").is_some());
        assert!(table.trigger("trg_posts").is_some());
    }

    #[test]
    fn expression_index_parts_are_not_reported_as_columns() {
        let index = DbIndex {
            name: "ix_lower_email".into(),
            columns: vec![
                DbIndexColumn {
                    value: "email".into(),
                    is_expression: false,
                },
                DbIndexColumn {
                    value: "lower(name)".into(),
                    is_expression: true,
                },
            ],
            ..Default::default()
        };
        assert_eq!(index.column_names(), vec!["email"]);
    }
}
