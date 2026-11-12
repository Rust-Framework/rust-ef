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
/// Semicolons inside single-quoted string literals (defaults, comments, CHECK
/// expressions) do not terminate a statement; `--` line comments are removed.
pub(super) fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut chars = sql.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_string = !in_string;
                current.push(c);
            }
            '-' if !in_string && chars.peek() == Some(&'-') => {
                // Skip the rest of the comment line.
                for next in chars.by_ref() {
                    if next == '\n' {
                        current.push('\n');
                        break;
                    }
                }
            }
            ';' if !in_string => {
                let statement = current.trim();
                if !statement.is_empty() {
                    statements.push(statement.to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }

    let statement = current.trim();
    if !statement.is_empty() {
        statements.push(statement.to_string());
    }
    statements
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
