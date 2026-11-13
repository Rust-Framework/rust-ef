//! Reversal of purely additive runtime diffs (EF-9 addendum).
//!
//! A runtime diff is deliberately conservative: it creates missing objects and
//! reports (without executing) destructive changes. Its inverse is therefore
//! only well-defined when **every** statement is additive — dropping a table or
//! column, or altering a column, loses information the plan does not carry (old
//! definitions, data). [`MigrationDiff::reverse`] implements exactly that
//! contract:
//!
//! 1. classify every statement and the rebuild list as additive or not;
//! 2. on any non-additive item, return a typed [`EFError::Migration`] that lists
//!    the irreversible items instead of emitting a misleading down script;
//! 3. on a purely additive plan, render the inverse statements in reverse
//!    order.
//!
//! Classification reads the plan's own SQL rather than adding a field to
//! [`MigrationDiff`]: the statements are produced by rust-ef's DDL generator, so
//! their shapes are closed, and a patch release must keep the public struct
//! literal-compatible.
//!
//! # Scope (what is reversible)
//!
//! | Forward statement | Reverse statement |
//! |---|---|
//! | `CREATE TABLE t (…)` | `DROP TABLE t` |
//! | `CREATE SEQUENCE s` | `DROP SEQUENCE IF EXISTS s` |
//! | `ALTER TABLE t ADD COLUMN c …` | `ALTER TABLE t DROP COLUMN c` |
//! | `CREATE [UNIQUE] INDEX i ON t (…)` | `DROP INDEX i` (MySQL: `DROP INDEX i ON t`) |
//! | `ALTER TABLE t ADD CONSTRAINT n FOREIGN KEY/CHECK …` | `ALTER TABLE t DROP CONSTRAINT/FOREIGN KEY/CHECK n` |
//! | `COMMENT ON TABLE/COLUMN …`, `ALTER TABLE t COMMENT = …` | omitted (metadata disappears with the created object) |
//!
//! Everything else — `DROP …` (reported as `SkippedRemoval` warnings by the
//! diff and never executed anyway), `ALTER COLUMN`/`MODIFY COLUMN`, table
//! options, table rebuilds, and `ManualAction` warnings — is reported as
//! irreversible.

use std::collections::HashSet;

use crate::error::{EFError, EFResult};
use crate::migration::dialect::MigrationDialect;

use super::{DiffWarning, MigrationDiff, WarningKind};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl MigrationDiff {
    /// Lists the plan's irreversible items (best effort, human-readable).
    ///
    /// A runtime diff is reversible only when this list is empty. The list is
    /// also printed by [`Self::report`] so an upgrade screen can show "what can
    /// be rolled back automatically" before applying the plan.
    ///
    /// Irreversible items are:
    ///
    /// - every SQLite table rebuild (the old table definition/data is replaced);
    /// - every `ManualAction` warning (the plan cannot perform that change
    ///   automatically, so it cannot be inverted automatically either);
    /// - every statement that is not one of the additive kinds listed in the
    ///   module documentation (e.g. `DROP …`, `ALTER COLUMN`, `MODIFY COLUMN`,
    ///   `ALTER TABLE … ENGINE=…`);
    /// - comment statements that target an object which the plan did not create
    ///   (changing an existing comment has no recorded previous value).
    pub fn irreversible_items(&self) -> Vec<String> {
        let parsed: Vec<ParsedStatement> = self
            .statements
            .iter()
            .map(|statement| ParsedStatement {
                statement: statement.clone(),
                kind: parse(statement),
            })
            .collect();

        let mut created = CreatedObjects::default();
        for parsed in &parsed {
            match &parsed.kind {
                StatementKind::CreateTable { table } => {
                    created.tables.insert(table.clone());
                }
                StatementKind::AddColumn { table, column } => {
                    created.columns.insert((table.clone(), column.clone()));
                }
                _ => {}
            }
        }

        let mut items: Vec<String> = Vec::new();
        for rebuild in &self.rebuilds {
            items.push(format!(
                "table rebuild required for {} (old definition/data is replaced)",
                rebuild.table
            ));
        }
        for warning in &self.warnings {
            if warning.kind == WarningKind::ManualAction {
                items.push(format!("manual action required: {}", warning.message));
            }
        }
        for parsed in &parsed {
            match &parsed.kind {
                StatementKind::CreateTable { .. }
                | StatementKind::CreateSequence { .. }
                | StatementKind::AddColumn { .. }
                | StatementKind::AddConstraint { .. }
                | StatementKind::CreateIndex { .. } => {}
                StatementKind::CommentOnTable { table } if created.tables.contains(table) => {}
                StatementKind::AlterTableComment { table } if created.tables.contains(table) => {}
                StatementKind::CommentOnColumn { table, column }
                    if created.tables.contains(table)
                        || created.columns.contains(&(table.clone(), column.clone())) => {}
                StatementKind::CommentOnTable { .. }
                | StatementKind::CommentOnColumn { .. }
                | StatementKind::AlterTableComment { .. } => {
                    items.push(format!(
                        "comment change on an existing object (previous value unknown): {}",
                        parsed.statement
                    ));
                }
                StatementKind::Irreversible => {
                    items.push(format!(
                        "destructive/unsupported statement: {}",
                        parsed.statement
                    ));
                }
            }
        }
        items
    }

    /// Builds the inverse plan for a purely additive runtime diff.
    ///
    /// Returns a typed [`EFError::Migration`] when the plan contains any
    /// irreversible item (see [`Self::irreversible_items`]); `dialect` is
    /// required because the inverse of `CREATE INDEX` differs between MySQL
    /// (`DROP INDEX i ON t`) and PostgreSQL/SQLite (`DROP INDEX i`).
    ///
    /// The returned plan has no rebuilds and a single informational warning
    /// recording its provenance. Apply it exactly like any other plan, e.g.
    /// with [`MigrationEngine::apply_diff`](super::super::engine::MigrationEngine::apply_diff)
    /// after taking a backup; a reverse plan is still a schema change.
    pub fn reverse(&self, dialect: MigrationDialect) -> EFResult<MigrationDiff> {
        let irreversible = self.irreversible_items();
        if !irreversible.is_empty() {
            return Err(EFError::migration(format!(
                "runtime diff is not reversible; irreversible items: {}",
                irreversible.join(" | ")
            )));
        }

        let mut statements: Vec<String> = Vec::new();
        for statement in self.statements.iter().rev() {
            let parsed = ParsedStatement {
                statement: statement.clone(),
                kind: parse(statement),
            };
            if let Some(sql) = render_reverse(&parsed, dialect)? {
                statements.push(sql);
            }
        }

        Ok(MigrationDiff {
            statements,
            warnings: vec![DiffWarning::info(
                WarningKind::ManualAction,
                None,
                "reverse plan generated from a purely additive runtime diff; \
                 review it and take a backup before applying",
            )],
            rebuilds: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Created-object bookkeeping
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct CreatedObjects {
    tables: HashSet<String>,
    columns: HashSet<(String, String)>,
}

// ---------------------------------------------------------------------------
// Statement parsing
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ParsedStatement {
    statement: String,
    kind: StatementKind,
}

/// Closed set of statement shapes the runtime-diff renderer can emit.
#[derive(Debug)]
enum StatementKind {
    CreateTable {
        /// Raw quoted identifier as it appears in the plan (e.g. `"items"`).
        table: String,
    },
    CreateSequence {
        sequence: String,
    },
    AddColumn {
        table: String,
        column: String,
    },
    AddConstraint {
        table: String,
        name: String,
        foreign_key: bool,
    },
    CreateIndex {
        name: String,
        table: String,
    },
    CommentOnTable {
        table: String,
    },
    CommentOnColumn {
        table: String,
        column: String,
    },
    AlterTableComment {
        table: String,
    },
    Irreversible,
}

fn parse(statement: &str) -> StatementKind {
    let statement = statement.trim();

    if let Some(rest) = strip_keyword(statement, "CREATE") {
        return parse_create(rest);
    }
    if let Some(rest) = strip_keyword(statement, "ALTER") {
        return parse_alter(rest);
    }
    if let Some(rest) = strip_keyword(statement, "COMMENT") {
        return parse_comment(rest);
    }
    StatementKind::Irreversible
}

fn parse_create(rest: &str) -> StatementKind {
    if let Some(rest) = strip_keyword(rest, "TABLE") {
        let rest = strip_if_not_exists(rest);
        if let Some((table, _)) = parse_qualified(rest) {
            return StatementKind::CreateTable { table };
        }
        return StatementKind::Irreversible;
    }
    if let Some(rest) = strip_keyword(rest, "SEQUENCE") {
        let rest = strip_if_not_exists(rest);
        if let Some((sequence, _)) = parse_qualified(rest) {
            return StatementKind::CreateSequence { sequence };
        }
        return StatementKind::Irreversible;
    }
    // `CREATE [UNIQUE] INDEX name ON table (…)`
    let after_unique = strip_keyword(rest, "UNIQUE");
    let index_rest = match after_unique {
        Some(after_unique) => strip_keyword(after_unique, "INDEX"),
        None => strip_keyword(rest, "INDEX"),
    };
    if let Some(index_rest) = index_rest {
        let index_rest = strip_if_not_exists(index_rest);
        if let Some((name, rest)) = parse_qualified(index_rest) {
            if let Some(rest) = strip_keyword(rest, "ON") {
                if let Some((table, _)) = parse_qualified(rest) {
                    return StatementKind::CreateIndex { name, table };
                }
            }
        }
    }
    StatementKind::Irreversible
}

fn parse_alter(rest: &str) -> StatementKind {
    let Some(rest) = strip_keyword(rest, "TABLE") else {
        return StatementKind::Irreversible;
    };
    let Some((table, rest)) = parse_qualified(rest) else {
        return StatementKind::Irreversible;
    };

    if let Some(rest) = strip_keyword(rest, "ADD") {
        if let Some(rest) = strip_keyword(rest, "COLUMN") {
            if let Some((column, _)) = parse_qualified(rest) {
                return StatementKind::AddColumn { table, column };
            }
        }
        if let Some(rest) = strip_keyword(rest, "CONSTRAINT") {
            if let Some((name, rest)) = parse_qualified(rest) {
                if starts_with_keyword(rest, "FOREIGN") {
                    return StatementKind::AddConstraint {
                        table,
                        name,
                        foreign_key: true,
                    };
                }
                if starts_with_keyword(rest, "CHECK") {
                    return StatementKind::AddConstraint {
                        table,
                        name,
                        foreign_key: false,
                    };
                }
            }
        }
    }

    // MySQL table comment: `ALTER TABLE t COMMENT = '…'`.
    if let Some(rest) = strip_keyword(rest, "COMMENT") {
        if rest.trim_start().starts_with('=') {
            return StatementKind::AlterTableComment { table };
        }
    }

    StatementKind::Irreversible
}

fn parse_comment(rest: &str) -> StatementKind {
    let Some(rest) = strip_keyword(rest, "ON") else {
        return StatementKind::Irreversible;
    };
    if let Some(rest) = strip_keyword(rest, "TABLE") {
        if let Some((table, _)) = parse_qualified(rest) {
            return StatementKind::CommentOnTable { table };
        }
        return StatementKind::Irreversible;
    }
    if let Some(rest) = strip_keyword(rest, "COLUMN") {
        if let Some(parts) = parse_qualified_parts(rest) {
            if parts.len() >= 2 {
                let column = parts.last().cloned().unwrap_or_default();
                let table = parts[..parts.len() - 1].join(".");
                return StatementKind::CommentOnColumn { table, column };
            }
        }
    }
    StatementKind::Irreversible
}

// ---------------------------------------------------------------------------
// Inverse rendering
// ---------------------------------------------------------------------------

fn render_reverse(parsed: &ParsedStatement, dialect: MigrationDialect) -> EFResult<Option<String>> {
    let statement = match &parsed.kind {
        StatementKind::CreateTable { table } => format!("DROP TABLE {table}"),
        StatementKind::CreateSequence { sequence } => {
            format!("DROP SEQUENCE IF EXISTS {sequence}")
        }
        StatementKind::AddColumn { table, column } => {
            format!("ALTER TABLE {table} DROP COLUMN {column}")
        }
        StatementKind::AddConstraint {
            table,
            name,
            foreign_key,
        } => match dialect {
            MigrationDialect::Sqlite => {
                return Err(EFError::migration(format!(
                    "SQLite cannot drop a constraint added by the plan \
                     (`ALTER TABLE … ADD CONSTRAINT` is not supported); \
                     statement: {}",
                    parsed.statement
                )));
            }
            MigrationDialect::MySql if *foreign_key => {
                format!("ALTER TABLE {table} DROP FOREIGN KEY {name}")
            }
            MigrationDialect::MySql => format!("ALTER TABLE {table} DROP CHECK {name}"),
            _ => format!("ALTER TABLE {table} DROP CONSTRAINT {name}"),
        },
        StatementKind::CreateIndex { name, table } => match dialect {
            MigrationDialect::MySql => format!("DROP INDEX {name} ON {table}"),
            _ => format!("DROP INDEX IF EXISTS {name}"),
        },
        // Metadata-only statements: the annotated object is created by the plan
        // (checked by `irreversible_items`) and is dropped by the reverse plan,
        // so there is nothing to restore.
        StatementKind::CommentOnTable { .. }
        | StatementKind::CommentOnColumn { .. }
        | StatementKind::AlterTableComment { .. } => return Ok(None),
        StatementKind::Irreversible => {
            return Err(EFError::migration(format!(
                "runtime diff statement is not reversible: {}",
                parsed.statement
            )));
        }
    };
    Ok(Some(statement))
}

// ---------------------------------------------------------------------------
// Tiny SQL-token helpers (quoted identifiers only, no expression parsing)
// ---------------------------------------------------------------------------

/// Matches `kw` case-insensitively at the start of `input` (after leading
/// whitespace) and returns the remainder only when a word boundary follows.
fn strip_keyword<'a>(input: &'a str, keyword: &str) -> Option<&'a str> {
    let input = input.trim_start();
    if input.len() < keyword.len() || !input[..keyword.len()].eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = &input[keyword.len()..];
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest)
    } else {
        None
    }
}

fn starts_with_keyword(input: &str, keyword: &str) -> bool {
    strip_keyword(input, keyword).is_some()
}

fn strip_if_not_exists(input: &str) -> &str {
    strip_keyword(input, "IF")
        .and_then(|rest| strip_keyword(rest, "NOT"))
        .and_then(|rest| strip_keyword(rest, "EXISTS"))
        .unwrap_or(input)
}

/// Parses one (possibly schema-qualified) SQL identifier and returns its raw
/// text exactly as it appears in the plan, quotes included.
fn parse_qualified(input: &str) -> Option<(String, &str)> {
    let parts = parse_qualified_parts(input)?;
    let consumed: usize = parts.iter().map(String::len).sum::<usize>() + parts.len() - 1;
    let rest = input.trim_start();
    let rest = rest.get(consumed..).unwrap_or("");
    Some((parts.join("."), rest))
}

/// Parses a dotted chain of identifiers (`"schema"."table"."column"`).
fn parse_qualified_parts(input: &str) -> Option<Vec<String>> {
    let mut rest = input.trim_start();
    let mut parts = Vec::new();
    loop {
        let (part, remaining) = parse_ident_part(rest)?;
        parts.push(part);
        rest = remaining.trim_start();
        match rest.strip_prefix('.') {
            Some(next) => rest = next.trim_start(),
            None => return Some(parts),
        }
    }
}

fn parse_ident_part(input: &str) -> Option<(String, &str)> {
    let first = input.chars().next()?;
    if first == '"' || first == '`' {
        let end = input[first.len_utf8()..].find(first)?;
        let end = end + first.len_utf8();
        let raw = &input[..=end];
        Some((raw.to_string(), &input[end + 1..]))
    } else {
        let end = input
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(input.len());
        if end == 0 {
            return None;
        }
        Some((input[..end].to_string(), &input[end..]))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::live_diff::TableRebuild;
    use crate::migration::MigrationDialect;

    fn diff_with(dialect: MigrationDialect, model_tables: Vec<&str>) -> MigrationDiff {
        use crate::migration::types::{ModelSnapshot, SnapshotColumn, SnapshotEntityType};
        use crate::migration::MigrationEngine;

        let entity_types = model_tables
            .into_iter()
            .map(|name| SnapshotEntityType {
                type_name: name.to_string(),
                table_name: name.to_string(),
                columns: vec![SnapshotColumn {
                    field_name: "id".to_string(),
                    column_name: "id".to_string(),
                    type_name: "i32".to_string(),
                    is_primary_key: true,
                    is_required: true,
                    is_auto_increment: true,
                    ..SnapshotColumn::default()
                }],
                ..SnapshotEntityType::default()
            })
            .collect();
        let model = ModelSnapshot {
            migration_id: "__test__".to_string(),
            entity_types,
        };
        MigrationEngine::new(dialect).diff(&model, &[])
    }

    #[test]
    fn additive_create_table_diff_is_reversible() {
        let diff = diff_with(MigrationDialect::Sqlite, vec!["items"]);
        assert!(!diff.statements.is_empty());
        assert!(diff.irreversible_items().is_empty());

        let reverse = diff.reverse(MigrationDialect::Sqlite).expect("reverse");
        assert_eq!(
            reverse.statements,
            vec!["DROP TABLE \"items\"".to_string()],
            "reverse up_sql:\n{}",
            reverse.up_sql()
        );
        assert!(reverse.rebuilds.is_empty());
        assert!(!reverse.warnings.is_empty(), "provenance warning expected");
    }

    #[test]
    fn reverse_renders_drop_in_reverse_order() {
        let reverse = MigrationDiff {
            statements: vec![
                "CREATE TABLE \"parents\" (\n    \"id\" INTEGER NOT NULL\n)".to_string(),
                "CREATE INDEX \"ix_parents_id\" ON \"parents\" (\"id\")".to_string(),
            ],
            warnings: vec![],
            rebuilds: vec![],
        }
        .reverse(MigrationDialect::Sqlite)
        .expect("reverse");

        assert_eq!(
            reverse.statements,
            vec![
                "DROP INDEX IF EXISTS \"ix_parents_id\"".to_string(),
                "DROP TABLE \"parents\"".to_string(),
            ]
        );
    }

    #[test]
    fn mysql_reverse_drops_index_with_table() {
        let reverse = MigrationDiff {
            statements: vec!["CREATE INDEX \"ix\" ON \"items\" (\"name\")".to_string()],
            warnings: vec![],
            rebuilds: vec![],
        }
        .reverse(MigrationDialect::MySql)
        .expect("reverse");
        assert_eq!(
            reverse.statements,
            vec!["DROP INDEX \"ix\" ON \"items\"".to_string()]
        );
    }

    #[test]
    fn reverse_add_column_and_constraint_forms() {
        let reverse = MigrationDiff {
            statements: vec![
                "ALTER TABLE \"events\" ADD COLUMN \"note\" VARCHAR(32) NULL".to_string(),
                "ALTER TABLE \"events\" ADD CONSTRAINT \"fk_events_user\" FOREIGN KEY (\"user_id\") REFERENCES \"users\" (\"id\")".to_string(),
                "ALTER TABLE \"events\" ADD CONSTRAINT \"ck_events_amount\" CHECK (amount > 0)".to_string(),
            ],
            warnings: vec![],
            rebuilds: vec![],
        }
        .reverse(MigrationDialect::Postgres)
        .expect("reverse");

        assert_eq!(
            reverse.statements,
            vec![
                "ALTER TABLE \"events\" DROP CONSTRAINT \"ck_events_amount\"".to_string(),
                "ALTER TABLE \"events\" DROP CONSTRAINT \"fk_events_user\"".to_string(),
                "ALTER TABLE \"events\" DROP COLUMN \"note\"".to_string(),
            ]
        );
    }

    #[test]
    fn mysql_reverse_uses_drop_foreign_key_and_drop_check() {
        let reverse = MigrationDiff {
            statements: vec![
                "ALTER TABLE `events` ADD CONSTRAINT `fk_user` FOREIGN KEY (`user_id`) REFERENCES `users` (`id`)".to_string(),
                "ALTER TABLE `events` ADD CONSTRAINT `ck_amount` CHECK (amount > 0)".to_string(),
            ],
            warnings: vec![],
            rebuilds: vec![],
        }
        .reverse(MigrationDialect::MySql)
        .expect("reverse");

        assert_eq!(
            reverse.statements,
            vec![
                "ALTER TABLE `events` DROP CHECK `ck_amount`".to_string(),
                "ALTER TABLE `events` DROP FOREIGN KEY `fk_user`".to_string(),
            ]
        );
    }

    #[test]
    fn destructive_statement_is_reported_and_reverse_errors() {
        let diff = MigrationDiff {
            statements: vec![
                "CREATE TABLE \"items\" (\n    \"id\" INTEGER NOT NULL\n)".to_string(),
                "ALTER TABLE \"items\" DROP COLUMN IF EXISTS \"legacy\"".to_string(),
            ],
            warnings: vec![],
            rebuilds: vec![],
        };

        let items = diff.irreversible_items();
        assert_eq!(items.len(), 1, "items: {items:?}");
        assert!(items[0].contains("legacy"), "items: {items:?}");

        match diff.reverse(MigrationDialect::Sqlite) {
            Err(EFError::Migration(message, _)) => {
                assert!(message.contains("irreversible"), "message: {message}");
            }
            other => panic!("expected a typed Migration error, got {other:?}"),
        }
    }

    #[test]
    fn table_rebuild_is_irreversible() {
        let diff = MigrationDiff {
            statements: vec![],
            warnings: vec![],
            rebuilds: vec![TableRebuild {
                table: "items".to_string(),
                ..TableRebuild::default()
            }],
        };
        assert_eq!(diff.irreversible_items().len(), 1);
        assert!(diff.reverse(MigrationDialect::Sqlite).is_err());
    }

    #[test]
    fn manual_action_warning_is_irreversible() {
        let diff = MigrationDiff {
            statements: vec![],
            warnings: vec![DiffWarning::warning(
                WarningKind::ManualAction,
                Some("items".to_string()),
                "primary-key change requires a manual migration",
            )],
            rebuilds: vec![],
        };
        assert_eq!(diff.irreversible_items().len(), 1);
        assert!(diff.reverse(MigrationDialect::Sqlite).is_err());
    }

    #[test]
    fn comment_on_created_object_is_reversible() {
        let diff = MigrationDiff {
            statements: vec![
                "CREATE TABLE \"items\" (\n    \"id\" INTEGER NOT NULL\n)".to_string(),
                "COMMENT ON TABLE \"items\" IS 'todo items'".to_string(),
                "COMMENT ON COLUMN \"items\".\"id\" IS 'primary key'".to_string(),
            ],
            warnings: vec![],
            rebuilds: vec![],
        };
        assert!(diff.irreversible_items().is_empty());
        let reverse = diff.reverse(MigrationDialect::Postgres).expect("reverse");
        assert_eq!(
            reverse.statements,
            vec!["DROP TABLE \"items\"".to_string()],
            "comments are metadata-only and disappear with the table"
        );
    }

    #[test]
    fn comment_on_existing_table_is_irreversible() {
        let diff = MigrationDiff {
            statements: vec!["COMMENT ON TABLE \"items\" IS 'new comment'".to_string()],
            warnings: vec![],
            rebuilds: vec![],
        };
        let items = diff.irreversible_items();
        assert_eq!(items.len(), 1, "items: {items:?}");
        assert!(diff.reverse(MigrationDialect::Postgres).is_err());
    }
}
