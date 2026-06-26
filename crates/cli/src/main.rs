//! rust-ef CLI — migration management for production deployments.

mod scaffold;

use clap::{Parser, Subcommand};
use rust_ef::error::{EFError, EFResult};
use rust_ef::metadata::EntityTypeMeta;
use rust_ef::migration::{
    parse_model_snapshot_json, MigrationDialect, MigrationEngine, MigrationStore, ModelSnapshot,
    PRODUCT_VERSION,
};
use rust_ef::provider::IDatabaseProvider;
use std::borrow::Cow;
use std::path::PathBuf;
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
    /// Revert the last applied migration
    Revert {
        #[arg(long)]
        connection: String,
        #[arg(long, value_enum, default_value = "auto")]
        provider: ProviderArg,
        #[arg(long, default_value = "Migrations")]
        dir: PathBuf,
    },
    /// Write a migration SQL script to stdout
    Script {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "Migrations")]
        dir: PathBuf,
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
            } => cmd_revert(&connection, provider, &dir).await,
            MigrationCommands::Script { name, dir } => cmd_script(&name, &dir),
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
    let text =
        std::fs::read_to_string(snapshot_path).map_err(|e| EFError::Migration(e.to_string()))?;
    let target_snapshot = parse_model_snapshot_json(&text)?.ok_or_else(|| {
        EFError::Migration("snapshot must describe at least one entity type".into())
    })?;
    let store = MigrationStore::new(dir);
    let previous = store.load_snapshot()?;
    let target_metas = snapshot_to_metas(&target_snapshot);
    let engine = MigrationEngine::new(dialect.to_migration_dialect());
    let migration = engine.generate(name, &target_metas, &previous)?;
    store.save(&migration)?;
    store.save_snapshot(&target_snapshot)?;
    println!("Created migration '{}' in {}", migration.id, dir.display());
    Ok(())
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

async fn cmd_revert(connection: &str, provider: ProviderArg, dir: &PathBuf) -> EFResult<()> {
    let p = create_provider(connection, provider)?;
    let dialect = p.migration_dialect();
    let store = MigrationStore::new(dir);
    let migrations = store.load_all()?;
    let engine = MigrationEngine::new(dialect);
    match engine.revert_last(&*p, &migrations).await? {
        Some(id) => {
            println!("Reverted migration '{}'.", id);
            Ok(())
        }
        None => {
            println!("No applied migrations to revert.");
            Ok(())
        }
    }
}

fn cmd_script(name: &str, dir: &PathBuf) -> EFResult<()> {
    let store = MigrationStore::new(dir);
    let migration = store.load(name)?;
    println!("-- Up: {}", migration.id);
    println!("{}", migration.up_sql);
    println!("-- Down: {}", migration.id);
    println!("{}", migration.down_sql);
    Ok(())
}

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
        ProviderArg::Postgres => rust_ef_postgres::introspection::introspect_postgres(connection)
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
            return Err(EFError::Configuration(
                "SQLite scaffold is not supported yet; use PostgreSQL or MySQL".into(),
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
            Ok(Arc::new(PostgresProvider::new(connection, 5)?))
        }
        ProviderArg::Mysql => {
            use rust_ef_mysql::MySqlProvider;
            Ok(Arc::new(MySqlProvider::new_lazy(connection)?))
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

fn snapshot_to_metas(snapshot: &ModelSnapshot) -> Vec<EntityTypeMeta> {
    snapshot
        .entity_types
        .iter()
        .map(|et| EntityTypeMeta {
            type_id: std::any::TypeId::of::<()>(),
            type_name: Cow::Owned(et.type_name.clone()),
            table_name: Cow::Owned(et.table_name.clone()),
            properties: et
                .columns
                .iter()
                .map(|c| rust_ef::metadata::PropertyMeta {
                    field_name: Cow::Owned(c.field_name.clone()),
                    column_name: Cow::Owned(c.column_name.clone()),
                    type_id: std::any::TypeId::of::<i32>(),
                    type_name: Cow::Owned(c.type_name.clone()),
                    is_primary_key: c.is_primary_key,
                    is_auto_increment: c.is_auto_increment,
                    is_required: c.is_required,
                    is_foreign_key: c.is_foreign_key,
                    is_concurrency_token: false,
                    max_length: c.max_length,
                    is_unique: false,
                    has_index: false,
                    is_not_mapped: false,
                })
                .collect(),
            navigations: Vec::new(),
            primary_keys: et
                .columns
                .iter()
                .filter(|c| c.is_primary_key)
                .map(|c| Cow::Owned(c.field_name.clone()))
                .collect(),
        })
        .collect()
}
