//! rust-ef CLI — migration management for production deployments.

mod scaffold;

use clap::{Parser, Subcommand};
use rust_ef::error::{EFError, EFResult};
use rust_ef::migration::{
    parse_model_snapshot_json, provider_capabilities, MigrationDialect, MigrationEngine,
    MigrationStore, ModelSnapshot, PRODUCT_VERSION,
};
use rust_ef::provider::IDatabaseProvider;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "rust-ef", about = "Rust Entity Framework CLI", version = PRODUCT_VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Migration commands
    Migration {
        #[command(subcommand)]
        command: MigrationCommands,
    },
    /// Database-first scaffolding
    Scaffold {
        #[command(subcommand)]
        command: ScaffoldCommands,
    },
}

#[derive(Subcommand)]
enum MigrationCommands {
    /// Create the __ef_migrations_history table in the database
    Init {
        #[arg(long)]
        connection: String,
        #[arg(long, value_enum, default_value = "auto")]
        provider: ProviderArg,
    },
    /// Generate a migration by diffing a model snapshot against the stored baseline
    Add {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "Migrations")]
        dir: PathBuf,
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long, value_enum, default_value = "sqlite")]
        dialect: DialectArg,
    },
    /// Apply pending migrations from the migrations directory
    Apply {
        #[arg(long)]
        connection: String,
        #[arg(long, value_enum, default_value = "auto")]
        provider: ProviderArg,
        #[arg(long, default_value = "Migrations")]
        dir: PathBuf,
    },
    /// List migrations (local and applied)
    List {
        #[arg(long)]
        connection: String,
        #[arg(long, value_enum, default_value = "auto")]
        provider: ProviderArg,
        #[arg(long, default_value = "Migrations")]
        dir: PathBuf,
    },
    /// Revert applied migrations. Without --target, reverts only the last.
    /// With `--target <Name>`, reverts all migrations after `<Name>` (exclusive).
    Revert {
        #[arg(long)]
        connection: String,
        #[arg(long, value_enum, default_value = "auto")]
        provider: ProviderArg,
        #[arg(long, default_value = "Migrations")]
        dir: PathBuf,
        /// Revert all migrations applied after this one (exclusive).
        #[arg(long)]
        target: Option<String>,
    },
    /// Write migration SQL script to stdout.
    /// Use `--name <Name>` for a single migration, or `--from`/`--to` for a range.
    Script {
        /// Print a single migration's up/down SQL by name.
        #[arg(long)]
        name: Option<String>,
        /// Generate a range script starting after this migration (exclusive).
        #[arg(long)]
        from: Option<String>,
        /// Generate a range script ending at this migration (inclusive).
        #[arg(long)]
        to: Option<String>,
        #[arg(long, default_value = "Migrations")]
        dir: PathBuf,
    },
    /// Render code-first DDL for one or all dialects from a model snapshot (no database).
    Ddl {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long, value_enum, default_value = "all")]
        dialect: DdlDialectArg,
        /// Write the script to a file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Print the provider capability matrix.
    Capabilities {
        #[arg(long, value_enum, default_value = "all")]
        dialect: DdlDialectArg,
    },
}

#[derive(Subcommand)]
enum ScaffoldCommands {
    /// Generate entity types from an existing database schema
    DbContext {
        #[arg(long)]
        connection: String,
        #[arg(long, value_enum, default_value = "auto")]
        provider: ProviderArg,
        #[arg(long, default_value = "Entities")]
        output: PathBuf,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ProviderArg {
    Auto,
    Sqlite,
    Postgres,
    Mysql,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum DialectArg {
    Sqlite,
    Postgres,
    Mysql,
}

impl DialectArg {
    fn to_migration_dialect(self) -> MigrationDialect {
        match self {
            DialectArg::Sqlite => MigrationDialect::Sqlite,
            DialectArg::Postgres => MigrationDialect::Postgres,
            DialectArg::Mysql => MigrationDialect::MySql,
        }
    }
}

/// Dialect selector for the read-only `ddl` / `capabilities` commands.
///
/// `All` renders the three dialects in a stable order (sqlite, postgres, mysql);
/// the single-dialect variants reuse [`DialectArg`]'s dialect mapping so the
/// `--dialect` > [`MigrationDialect`] conversion has one source of truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum DdlDialectArg {
    All,
    Sqlite,
    Postgres,
    Mysql,
}

impl DdlDialectArg {
    /// Dialects this selector expands to, in rendering order.
    fn selected(self) -> Vec<MigrationDialect> {
        match self {
            DdlDialectArg::All => vec![
                DialectArg::Sqlite.to_migration_dialect(),
                DialectArg::Postgres.to_migration_dialect(),
                DialectArg::Mysql.to_migration_dialect(),
            ],
            DdlDialectArg::Sqlite => vec![DialectArg::Sqlite.to_migration_dialect()],
            DdlDialectArg::Postgres => vec![DialectArg::Postgres.to_migration_dialect()],
            DdlDialectArg::Mysql => vec![DialectArg::Mysql.to_migration_dialect()],
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), EFError> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Migration { command } => match command {
            MigrationCommands::Init {
                connection,
                provider,
            } => cmd_init(&connection, provider).await,
            MigrationCommands::Add {
                name,
                dir,
                snapshot,
                dialect,
            } => cmd_add(&name, &dir, &snapshot, dialect),
            MigrationCommands::Apply {
                connection,
                provider,
                dir,
            } => cmd_apply(&connection, provider, &dir).await,
            MigrationCommands::List {
                connection,
                provider,
                dir,
            } => cmd_list(&connection, provider, &dir).await,
            MigrationCommands::Revert {
                connection,
                provider,
                dir,
                target,
            } => cmd_revert(&connection, provider, &dir, target.as_deref()).await,
            MigrationCommands::Script {
                name,
                from,
                to,
                dir,
            } => cmd_script(name.as_deref(), from.as_deref(), to.as_deref(), &dir),
            MigrationCommands::Ddl {
                snapshot,
                dialect,
                out,
            } => cmd_ddl(&snapshot, dialect, out.as_deref()),
            MigrationCommands::Capabilities { dialect } => cmd_capabilities(dialect),
        },
        Commands::Scaffold { command } => match command {
            ScaffoldCommands::DbContext {
                connection,
                provider,
                output,
            } => cmd_scaffold_dbcontext(&connection, provider, &output).await,
        },
    }
}

async fn cmd_init(connection: &str, provider: ProviderArg) -> EFResult<()> {
    let p = create_provider(connection, provider)?;
    let dialect = p.migration_dialect();
    MigrationEngine::new(dialect)
        .ensure_history_table(&*p)
        .await?;
    println!("Migration history table ensured.");
    Ok(())
}

fn cmd_add(
    name: &str,
    dir: &PathBuf,
    snapshot_path: &PathBuf,
    dialect: DialectArg,
) -> EFResult<()> {
    let target_snapshot = load_snapshot(snapshot_path)?;
    let baseline = dir.join("model_snapshot.json");
    if snapshot_path == &baseline {
        return Err(EFError::migration(
            "--snapshot must point at the *current* model snapshot, not the \
             Migrations/model_snapshot.json baseline (diffing a file against \
             itself yields an empty migration)",
        ));
    }
    let store = MigrationStore::new(dir);
    let previous = store.load_snapshot()?;
    let engine = MigrationEngine::new(dialect.to_migration_dialect());
    let id = timestamped_migration_id(name)?;
    let mut migration = engine.generate_from_snapshot(&id, &target_snapshot, &previous)?;
    migration.description = name.to_string();
    store.save(&migration)?;
    store.save_snapshot(&target_snapshot)?;
    println!("Created migration '{}' in {}", migration.id, dir.display());
    Ok(())
}

/// Builds a sortable migration id: `{unix_seconds}_{sanitized-name}`.
///
/// `MigrationStore::load_all` sorts by id, so the timestamp prefix guarantees
/// creation order even when names are not alphabetical.
fn timestamped_migration_id(name: &str) -> EFResult<String> {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.trim_matches('_').is_empty() {
        return Err(EFError::migration(
            "migration name must contain at least one alphanumeric character",
        ));
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(format!("{seconds:010}_{sanitized}"))
}

async fn cmd_apply(connection: &str, provider: ProviderArg, dir: &PathBuf) -> EFResult<()> {
    let p = create_provider(connection, provider)?;
    let dialect = p.migration_dialect();
    let store = MigrationStore::new(dir);
    let migrations = store.load_all()?;
    let engine = MigrationEngine::new(dialect);
    let applied = engine.apply_pending(&*p, &migrations).await?;
    println!("Applied {} migration(s).", applied);
    Ok(())
}

async fn cmd_list(connection: &str, provider: ProviderArg, dir: &PathBuf) -> EFResult<()> {
    let p = create_provider(connection, provider)?;
    let dialect = p.migration_dialect();
    let store = MigrationStore::new(dir);
    let local = store.load_all()?;
    let engine = MigrationEngine::new(dialect);
    let applied = engine.get_applied_migrations(&*p).await?;
    let applied_ids: std::collections::HashSet<_> =
        applied.iter().map(|e| e.migration_id.as_str()).collect();

    println!("Local migrations ({}):", local.len());
    for m in &local {
        let status = if applied_ids.contains(m.id.as_str()) {
            "applied"
        } else {
            "pending"
        };
        println!("  [{}] {}", status, m.id);
    }
    if applied.is_empty() {
        println!("No migrations recorded in database.");
    } else {
        println!("Applied in database ({}):", applied.len());
        for e in &applied {
            println!("  {} (v{})", e.migration_id, e.product_version);
        }
    }
    Ok(())
}

async fn cmd_revert(
    connection: &str,
    provider: ProviderArg,
    dir: &PathBuf,
    target: Option<&str>,
) -> EFResult<()> {
    let p = create_provider(connection, provider)?;
    let dialect = p.migration_dialect();
    let store = MigrationStore::new(dir);
    let migrations = store.load_all()?;
    let engine = MigrationEngine::new(dialect);
    match target {
        Some(t) => {
            let reverted = engine.revert_to_target(&*p, &migrations, Some(t)).await?;
            if reverted.is_empty() {
                println!("No migrations to revert after '{}'.", t);
            } else {
                println!("Reverted {} migration(s):", reverted.len());
                for id in &reverted {
                    println!("  - {}", id);
                }
            }
            Ok(())
        }
        None => match engine.revert_last(&*p, &migrations).await? {
            Some(id) => {
                println!("Reverted migration '{}'.", id);
                Ok(())
            }
            None => {
                println!("No applied migrations to revert.");
                Ok(())
            }
        },
    }
}

fn cmd_script(
    name: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
    dir: &PathBuf,
) -> EFResult<()> {
    let store = MigrationStore::new(dir);
    if let Some(n) = name {
        let migration = store.load(n)?;
        println!("-- Up: {}", migration.id);
        println!("{}", migration.up_sql);
        println!("-- Down: {}", migration.id);
        println!("{}", migration.down_sql);
        return Ok(());
    }
    let migrations = store.load_all()?;
    let sql = MigrationEngine::generate_script(&migrations, from, to)?;
    print!("{}", sql);
    Ok(())
}

/// Reads and parses a model snapshot file.
///
/// Shared by `migration add` and `migration ddl` so both commands accept the
/// same snapshot format and report the same errors.
fn load_snapshot(path: &Path) -> EFResult<ModelSnapshot> {
    let text = std::fs::read_to_string(path).map_err(|e| EFError::migration(e.to_string()))?;
    parse_model_snapshot_json(&text)?
        .ok_or_else(|| EFError::migration("snapshot must describe at least one entity type"))
}

/// Renders code-first DDL for the selected dialects without touching a
/// database: the diff is computed against an empty live schema, so every model
/// table becomes a `CREATE TABLE` (the "fresh database" path, which does not
/// emit `__ef_migrations_history`), followed by the diff report and the
/// provider capability matrix.
fn cmd_ddl(snapshot_path: &Path, dialect: DdlDialectArg, out: Option<&Path>) -> EFResult<()> {
    let snapshot = load_snapshot(snapshot_path)?;

    let mut rendered = String::new();
    for selected in dialect.selected() {
        let capabilities = provider_capabilities(selected);
        let diff = MigrationEngine::new(selected).diff(&snapshot, &[]);

        rendered.push_str(&format!("-- dialect: {}\n", capabilities.dialect_name()));
        let sql = diff.up_sql();
        if sql.trim().is_empty() {
            rendered.push_str("-- no statements\n");
        } else {
            rendered.push_str(&sql);
        }
        rendered.push('\n');
        rendered.push_str(&diff.report());
        rendered.push('\n');
        rendered.push_str(&capabilities.report());
        rendered.push('\n');
    }

    match out {
        Some(path) => std::fs::write(path, rendered)
            .map_err(|e| EFError::migration(format!("cannot write {}: {e}", path.display())))?,
        None => print!("{rendered}"),
    }
    Ok(())
}

/// Prints the provider capability matrix for the selected dialects.
fn cmd_capabilities(dialect: DdlDialectArg) -> EFResult<()> {
    for selected in dialect.selected() {
        print!("{}", provider_capabilities(selected).report());
    }
    Ok(())
}

// Scaffolding still uses the legacy free introspection functions (they take a
// connection string and configure TLS per call); the canonical entry point is
// `IDatabaseProvider::introspect`. The allow documents the remaining call sites
// instead of letting them emit deprecation warnings.
#[allow(deprecated)]
async fn cmd_scaffold_dbcontext(
    connection: &str,
    provider: ProviderArg,
    #[allow(clippy::ptr_arg)] output: &PathBuf,
) -> EFResult<()> {
    let kind = match provider {
        ProviderArg::Auto => detect_provider(connection),
        other => other,
    };
    let tables: Vec<scaffold::ScaffoldTable> = match kind {
        ProviderArg::Postgres => rust_ef_postgres::introspection::introspect_postgres(
            connection,
            rust_ef_postgres::PgTlsMode::Disable,
        )
        .await?
        .into_iter()
        .map(|t| scaffold::ScaffoldTable {
            name: t.name,
            columns: t
                .columns
                .into_iter()
                .map(|c| scaffold::ScaffoldColumn {
                    name: c.name,
                    data_type: c.data_type,
                    is_nullable: c.is_nullable,
                    is_primary_key: c.is_primary_key,
                    max_length: c.max_length,
                })
                .collect(),
        })
        .collect(),
        ProviderArg::Mysql => rust_ef_mysql::introspection::introspect_mysql(connection)
            .await?
            .into_iter()
            .map(|t| scaffold::ScaffoldTable {
                name: t.name,
                columns: t
                    .columns
                    .into_iter()
                    .map(|c| scaffold::ScaffoldColumn {
                        name: c.name,
                        data_type: c.data_type,
                        is_nullable: c.is_nullable,
                        is_primary_key: c.is_primary_key,
                        max_length: c.max_length,
                    })
                    .collect(),
            })
            .collect(),
        ProviderArg::Sqlite => {
            return Err(EFError::configuration(
                "SQLite scaffold is not supported yet; use PostgreSQL or MySQL",
            ));
        }
        ProviderArg::Auto => unreachable!(),
    };

    if tables.is_empty() {
        println!("No user tables found; nothing generated.");
        return Ok(());
    }

    scaffold::write_entities(output, &tables)?;
    println!(
        "Scaffolded {} entity type(s) to {}",
        tables.len(),
        output.display()
    );
    Ok(())
}

fn create_provider(
    connection: &str,
    provider: ProviderArg,
) -> EFResult<Arc<dyn IDatabaseProvider>> {
    let kind = match provider {
        ProviderArg::Auto => detect_provider(connection),
        other => other,
    };
    match kind {
        ProviderArg::Sqlite => {
            use rust_ef_sqlite::SqliteProvider;
            Ok(Arc::new(SqliteProvider::new(connection)?))
        }
        ProviderArg::Postgres => {
            use rust_ef_postgres::PostgresProvider;
            Ok(Arc::new(PostgresProvider::new_insecure(connection, 5)?))
        }
        ProviderArg::Mysql => {
            use rust_ef_mysql::MySqlProvider;
            Ok(Arc::new(MySqlProvider::new_lazy_insecure(connection)?))
        }
        ProviderArg::Auto => unreachable!(),
    }
}

fn detect_provider(connection: &str) -> ProviderArg {
    let lower = connection.to_ascii_lowercase();
    if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
        ProviderArg::Postgres
    } else if lower.starts_with("mysql://") {
        ProviderArg::Mysql
    } else {
        ProviderArg::Sqlite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_dialect_arg_all_selects_every_dialect_in_stable_order() {
        assert!(matches!(
            DdlDialectArg::All.selected().as_slice(),
            [
                MigrationDialect::Sqlite,
                MigrationDialect::Postgres,
                MigrationDialect::MySql
            ]
        ));
    }

    #[test]
    fn ddl_dialect_arg_single_selects_exactly_that_dialect() {
        assert!(matches!(
            DdlDialectArg::Sqlite.selected().as_slice(),
            [MigrationDialect::Sqlite]
        ));
        assert!(matches!(
            DdlDialectArg::Postgres.selected().as_slice(),
            [MigrationDialect::Postgres]
        ));
        assert!(matches!(
            DdlDialectArg::Mysql.selected().as_slice(),
            [MigrationDialect::MySql]
        ));
    }

    #[test]
    fn parses_ddl_command_with_default_dialect_and_stdout() {
        let cli = Cli::try_parse_from(["rust-ef", "migration", "ddl", "--snapshot", "snap.json"])
            .unwrap();
        match cli.command {
            Commands::Migration {
                command:
                    MigrationCommands::Ddl {
                        snapshot,
                        dialect,
                        out,
                    },
            } => {
                assert_eq!(snapshot, PathBuf::from("snap.json"));
                assert_eq!(dialect, DdlDialectArg::All);
                assert_eq!(out, None);
            }
            _ => panic!("expected `migration ddl`"),
        }
    }

    #[test]
    fn parses_ddl_command_with_dialect_and_out_file() {
        let cli = Cli::try_parse_from([
            "rust-ef",
            "migration",
            "ddl",
            "--snapshot",
            "snap.json",
            "--dialect",
            "mysql",
            "--out",
            "ddl.sql",
        ])
        .unwrap();
        match cli.command {
            Commands::Migration {
                command:
                    MigrationCommands::Ddl {
                        snapshot,
                        dialect,
                        out,
                    },
            } => {
                assert_eq!(snapshot, PathBuf::from("snap.json"));
                assert_eq!(dialect, DdlDialectArg::Mysql);
                assert_eq!(out, Some(PathBuf::from("ddl.sql")));
            }
            _ => panic!("expected `migration ddl`"),
        }
    }

    #[test]
    fn ddl_requires_a_snapshot() {
        assert!(Cli::try_parse_from(["rust-ef", "migration", "ddl"]).is_err());
    }

    #[test]
    fn parses_capabilities_command() {
        let cli = Cli::try_parse_from([
            "rust-ef",
            "migration",
            "capabilities",
            "--dialect",
            "postgres",
        ])
        .unwrap();
        match cli.command {
            Commands::Migration {
                command: MigrationCommands::Capabilities { dialect },
            } => assert_eq!(dialect, DdlDialectArg::Postgres),
            _ => panic!("expected `migration capabilities`"),
        }
    }
}
