//! Migration history table management and filesystem migration store.

use super::dialect::MigrationDialect;
use super::snapshot_io::migration_io_err;
use super::types::{Migration, ModelSnapshot};
use crate::error::{EFError, EFResult};
use std::fs;
use std::path::{Path, PathBuf};

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

pub(super) fn seed_insert_sql(
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
///
/// The splitter understands the lexical cases a migration script actually
/// contains: semicolons inside single-quoted literals (including doubled `''`
/// and, for MySQL, backslash escapes), `--` line comments, `/* ... */` block
/// comments, and PostgreSQL `$tag$ ... $tag$` dollar-quoted strings (function
/// bodies). Comments are dropped, statement text is trimmed, and empty
/// statements are skipped.
pub(super) fn split_sql_statements(sql: &str, dialect: MigrationDialect) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut dollar_tag: Option<String> = None;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
                current.push('\n');
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if c == '*' && chars.get(i + 1) == Some(&'/') {
                in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if let Some(tag) = &dollar_tag {
            let tag_chars: Vec<char> = tag.chars().collect();
            if c == '$' && chars[i..].starts_with(&tag_chars) {
                current.push_str(tag);
                i += tag_chars.len();
                dollar_tag = None;
            } else {
                current.push(c);
                i += 1;
            }
            continue;
        }
        if in_string {
            match c {
                // MySQL: backslash escapes the next character inside a literal.
                '\\' if dialect == MigrationDialect::MySql => {
                    current.push(c);
                    if let Some(next) = chars.get(i + 1) {
                        current.push(*next);
                        i += 1;
                    }
                }
                // A doubled quote is an escaped quote in every supported dialect.
                '\'' if chars.get(i + 1) == Some(&'\'') => {
                    current.push_str("''");
                    i += 1;
                }
                '\'' => {
                    in_string = false;
                    current.push(c);
                }
                _ => current.push(c),
            }
            i += 1;
            continue;
        }

        match c {
            '\'' => {
                in_string = true;
                current.push(c);
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                in_line_comment = true;
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                in_block_comment = true;
                i += 1;
            }
            '$' if dialect == MigrationDialect::Postgres => match dollar_quote_tag(&chars, i) {
                Some(tag) => {
                    current.push_str(&tag);
                    i += tag.chars().count() - 1;
                    dollar_tag = Some(tag);
                }
                None => current.push(c),
            },
            // MySQL also accepts `#` line comments.
            '#' if dialect == MigrationDialect::MySql => {
                in_line_comment = true;
            }
            ';' => {
                let statement = current.trim();
                if !statement.is_empty() {
                    statements.push(statement.to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
        i += 1;
    }

    let statement = current.trim();
    if !statement.is_empty() {
        statements.push(statement.to_string());
    }
    statements
}

/// Returns the full `$tag$` delimiter starting at `start`, if `chars[start..]`
/// opens a PostgreSQL dollar-quoted string (`$$` or `$identifier$`).
///
/// PostgreSQL only treats `$...$` as a dollar quote when the tag is empty or a
/// valid identifier (it cannot start with a digit), which keeps positional
/// parameters such as `$1` out of this path.
fn dollar_quote_tag(chars: &[char], start: usize) -> Option<String> {
    let rest = &chars[start..];
    let close = rest.iter().skip(1).position(|c| *c == '$').map(|p| p + 1)?;
    let tag: String = rest[..=close].iter().collect();
    let name = &tag[1..tag.len() - 1];
    let valid_ident = match name.chars().next() {
        None => true,
        Some(first) => {
            (first.is_ascii_alphabetic() || first == '_')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
    };
    valid_ident.then_some(tag)
}

/// Validates a migration id used as a directory name and history key.
///
/// Rejects empty ids, path separators, `..`, drive letters, control characters,
/// and ids longer than the history table's 150-character column.
pub fn validate_migration_id(id: &str) -> EFResult<()> {
    if id.is_empty() {
        return Err(EFError::migration("migration id must not be empty"));
    }
    if id.len() > 150 {
        return Err(EFError::migration(format!(
            "migration id '{id}' exceeds 150 characters"
        )));
    }
    if id == "." || id == ".." {
        return Err(EFError::migration(format!("invalid migration id '{id}'")));
    }
    if id
        .chars()
        .any(|c| c == '/' || c == '\\' || c == ':' || c.is_control())
    {
        return Err(EFError::migration(format!(
            "migration id '{id}' must not contain path separators, ':' or control characters"
        )));
    }
    // Windows strips trailing dots/spaces from directory names, which would
    // make the on-disk id differ from the history key.
    if id.ends_with('.') || id.ends_with(' ') {
        return Err(EFError::migration(format!(
            "migration id '{id}' must not end with '.' or a space"
        )));
    }
    Ok(())
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
    ///
    /// `id` must be a safe, ordered directory name (no path separators/`..`);
    /// use a timestamp prefix so [`Self::load_all`] applies migrations in
    /// creation order.
    pub fn save(&self, migration: &Migration) -> EFResult<()> {
        validate_migration_id(&migration.id)?;
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
        validate_migration_id(id)?;
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
        super::snapshot::write_snapshot(self.root.join("model_snapshot.json"), snapshot)
    }

    pub fn load_snapshot(&self) -> EFResult<Option<ModelSnapshot>> {
        let path = self.root.join("model_snapshot.json");
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(path).map_err(migration_io_err)?;
        super::snapshot_io::parse_model_snapshot_json(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(sql: &str, dialect: MigrationDialect) -> Vec<String> {
        split_sql_statements(sql, dialect)
    }

    #[test]
    fn splits_plain_statements_and_drops_empty_ones() {
        let out = split(
            "CREATE TABLE a (id INT); ;CREATE TABLE b (id INT)",
            MigrationDialect::Sqlite,
        );
        assert_eq!(
            out,
            vec!["CREATE TABLE a (id INT)", "CREATE TABLE b (id INT)"]
        );
    }

    #[test]
    fn semicolon_inside_string_literal_does_not_split() {
        let out = split(
            "INSERT INTO t VALUES ('a;b');SELECT 1",
            MigrationDialect::MySql,
        );
        assert_eq!(out, vec!["INSERT INTO t VALUES ('a;b')", "SELECT 1"]);
    }

    #[test]
    fn doubled_quote_is_an_escaped_quote() {
        let out = split(
            "INSERT INTO t VALUES ('it''s;ok');SELECT 1",
            MigrationDialect::Postgres,
        );
        assert_eq!(out, vec!["INSERT INTO t VALUES ('it''s;ok')", "SELECT 1"]);
    }

    #[test]
    fn mysql_backslash_escape_keeps_the_string_open() {
        let out = split(
            "INSERT INTO t VALUES ('it\\'s;ok');SELECT 1",
            MigrationDialect::MySql,
        );
        assert_eq!(out, vec!["INSERT INTO t VALUES ('it\\'s;ok')", "SELECT 1"]);
    }

    #[test]
    fn postgres_backslash_is_not_an_escape() {
        // `'a\'` closes the literal; the following `;` separates statements.
        let out = split("SELECT 'a\\';SELECT 1", MigrationDialect::Postgres);
        assert_eq!(out, vec!["SELECT 'a\\'", "SELECT 1"]);
    }

    #[test]
    fn line_comments_are_stripped() {
        let out = split(
            "-- note; still comment\nSELECT 1;",
            MigrationDialect::Postgres,
        );
        assert_eq!(out, vec!["SELECT 1"]);
    }

    #[test]
    fn block_comments_are_stripped() {
        let out = split("/* a;b */ SELECT 1; /* tail */", MigrationDialect::Sqlite);
        assert_eq!(out, vec!["SELECT 1"]);
    }

    #[test]
    fn postgres_dollar_quoted_body_keeps_semicolons() {
        let out = split(
            "CREATE FUNCTION f() RETURNS void AS $$ BEGIN PERFORM 1; END $$ LANGUAGE plpgsql; SELECT 2",
            MigrationDialect::Postgres,
        );
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("PERFORM 1; END"));
        assert_eq!(out[1], "SELECT 2");
    }

    #[test]
    fn dollar_inside_a_literal_is_not_a_dollar_quote() {
        let out = split("SELECT '$';SELECT 1", MigrationDialect::Postgres);
        assert_eq!(out, vec!["SELECT '$'", "SELECT 1"]);
    }

    #[test]
    fn postgres_positional_parameters_are_not_dollar_quotes() {
        let out = split("SELECT $1, $2;SELECT 3", MigrationDialect::Postgres);
        assert_eq!(out, vec!["SELECT $1, $2", "SELECT 3"]);
    }

    #[test]
    fn mysql_hash_comment_is_stripped() {
        let out = split("# note; here\nSELECT 1;", MigrationDialect::MySql);
        assert_eq!(out, vec!["SELECT 1"]);
    }
}
