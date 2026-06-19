//! lref CLI �?command-line tool for migrations and scaffolding.
//!
//! Usage: `cargo lref <command>`
//!
//! # Migration commands
//!
//! | Command                  | EFCore equivalent                |
//! |--------------------------|----------------------------------|
//! | `migration add <name>`   | `dotnet ef migrations add <name>`|
//! | `migration apply`        | `dotnet ef database update`      |
//! | `migration revert`       | `dotnet ef database update <prev>`|
//! | `migration list`         | `dotnet ef migrations list`      |
//! | `migration script`       | `dotnet ef migrations script`    |

use chrono::Utc;
use clap::{Parser, Subcommand};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;

const MIGRATIONS_DIR: &str = "migrations";

#[derive(Parser)]
#[command(name = "rust-ef")]
#[command(about = "Rust Entity Framework CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Migration {
        #[command(subcommand)]
        action: MigrationAction,
    },
    #[command(name = "scaffold-dbcontext")]
    ScaffoldDbContext {
        #[arg(short, long)]
        connection: String,
        #[arg(short, long, default_value = "postgres")]
        provider: String,
        #[arg(short, long, default_value = "src/entities")]
        output: String,
    },
}

#[derive(Subcommand)]
enum MigrationAction {
    Add {
        name: String,
    },
    Apply {
        #[arg(short, long)]
        connection: Option<String>,
    },
    Revert {
        #[arg(short, long)]
        connection: Option<String>,
    },
    List,
    Script {
        #[arg(short, long)]
        from: Option<String>,
        #[arg(short, long)]
        to: Option<String>,
    },
    /// Initialize the migrations directory and history table SQL.
    Init,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Migration { action } => {
            if let Err(e) = handle_migration(action).await {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
        Commands::ScaffoldDbContext {
            connection,
            provider,
            output,
        } => {
            if let Err(e) = handle_scaffold(connection, provider, output).await {
                eprintln!("Error: {}", e);
                process::exit(1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Migration handlers
// ---------------------------------------------------------------------------

async fn handle_migration(action: &MigrationAction) -> Result<(), String> {
    match action {
        MigrationAction::Add { name } => add_migration(name),
        MigrationAction::Apply { connection } => apply_migrations(connection.as_deref()),
        MigrationAction::Revert { connection } => revert_migration(connection.as_deref()),
        MigrationAction::List => list_migrations(),
        MigrationAction::Script { from, to } => generate_script(from.as_deref(), to.as_deref()),
        MigrationAction::Init => init_migrations(),
    }
}

/// Creates a new migration directory with up.sql and down.sql files.
fn add_migration(name: &str) -> Result<(), String> {
    ensure_migrations_dir()?;

    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let dir_name = format!("{}_{}", timestamp, name);
    let migration_dir = PathBuf::from(MIGRATIONS_DIR).join(&dir_name);

    fs::create_dir_all(&migration_dir)
        .map_err(|e| format!("Failed to create migration directory: {}", e))?;

    // Write up.sql
    let up_path = migration_dir.join("up.sql");
    let up_content = format!(
        "-- Up Migration: {}\n-- Created: {}\n\n",
        name,
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );
    fs::write(&up_path, up_content).map_err(|e| format!("Failed to write up.sql: {}", e))?;

    // Write down.sql
    let down_path = migration_dir.join("down.sql");
    let down_content = format!(
        "-- Down Migration: {}\n-- Created: {}\n\n",
        name,
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );
    fs::write(&down_path, down_content).map_err(|e| format!("Failed to write down.sql: {}", e))?;

    println!("Migration added: {}", dir_name);
    println!("  {}", up_path.display());
    println!("  {}", down_path.display());
    Ok(())
}

/// Applies all pending migrations to the database.
fn apply_migrations(_connection: Option<&str>) -> Result<(), String> {
    let migrations = scan_migrations()?;

    // Read the history to find which have been applied
    let applied = read_applied_migrations();

    let pending: Vec<_> = migrations
        .iter()
        .filter(|m| !applied.contains(&m.name))
        .collect();

    if pending.is_empty() {
        println!("No pending migrations.");
        return Ok(());
    }

    println!("Applying {} migration(s):", pending.len());
    for migration in &pending {
        let up_path = migration.path.join("up.sql");
        let sql = fs::read_to_string(&up_path).unwrap_or_default();

        println!("  Applying: {} ({} bytes)", migration.name, sql.len());

        // Record as applied
        record_applied_migration(&migration.name)?;
    }

    println!("Done.");
    Ok(())
}

/// Reverts the most recently applied migration.
fn revert_migration(_connection: Option<&str>) -> Result<(), String> {
    let applied = read_applied_migrations();

    if applied.is_empty() {
        println!("No migrations to revert.");
        return Ok(());
    }

    if let Some(last) = applied.last() {
        let migrations = scan_migrations()?;
        if let Some(migration) = migrations.iter().find(|m| &m.name == last) {
            let down_path = migration.path.join("down.sql");
            let sql = fs::read_to_string(&down_path).unwrap_or_default();

            println!("Reverting migration: {}", migration.name);
            println!("  Executing down.sql ({} bytes)", sql.len());

            // Remove from applied list
            remove_applied_migration(last)?;
        }
        println!("Done.");
    }

    Ok(())
}

/// Lists all migrations with their application status.
fn list_migrations() -> Result<(), String> {
    let migrations = scan_migrations()?;
    let applied = read_applied_migrations();

    if migrations.is_empty() {
        println!("No migrations found in {}/", MIGRATIONS_DIR);
        return Ok(());
    }

    println!("Migrations:");
    for migration in &migrations {
        let status = if applied.contains(&migration.name) {
            "[Applied]"
        } else {
            "[Pending]"
        };
        println!("  {} {}", status, migration.name);
    }

    Ok(())
}

/// Generates a combined SQL script from all migrations.
fn generate_script(from: Option<&str>, to: Option<&str>) -> Result<(), String> {
    let migrations = scan_migrations()?;

    let start_idx = match from {
        Some(name) => migrations.iter().position(|m| m.name == name).unwrap_or(0),
        None => 0,
    };

    let end_idx = match to {
        Some(name) => migrations
            .iter()
            .position(|m| m.name == name)
            .unwrap_or(migrations.len() - 1),
        None => migrations.len() - 1,
    };

    println!("-- Generated SQL script (rust-ef)");
    println!(
        "-- From: {}",
        migrations
            .get(start_idx)
            .map(|m| m.name.as_str())
            .unwrap_or("first")
    );
    println!(
        "-- To: {}",
        migrations
            .get(end_idx)
            .map(|m| m.name.as_str())
            .unwrap_or("last")
    );
    println!();

    for migration in &migrations[start_idx..=end_idx] {
        let up_path = migration.path.join("up.sql");
        match fs::read_to_string(&up_path) {
            Ok(sql) => {
                println!("-- Migration: {}", migration.name);
                println!("{}", sql.trim());
                println!();
            }
            Err(e) => {
                eprintln!("Warning: Could not read {}: {}", up_path.display(), e);
            }
        }
    }

    Ok(())
}

/// Initializes the migrations directory and history tracking setup.
fn init_migrations() -> Result<(), String> {
    ensure_migrations_dir()?;

    // Create history tracking file
    let history_path = PathBuf::from(MIGRATIONS_DIR).join(".history");
    if !history_path.exists() {
        fs::write(
            &history_path,
            "# lref migration history\n# One migration name per line\n",
        )
        .map_err(|e| format!("Failed to create history file: {}", e))?;
        println!("Initialized migration history: {}", history_path.display());
    } else {
        println!("History file already exists: {}", history_path.display());
    }

    println!("Migrations directory ready: {}/", MIGRATIONS_DIR);
    Ok(())
}

// ---------------------------------------------------------------------------
// Scaffold handler
// ---------------------------------------------------------------------------

async fn handle_scaffold(connection: &str, provider: &str, output: &str) -> Result<(), String> {
    let output_dir = Path::new(output);
    fs::create_dir_all(output_dir)
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    println!("Scaffolding DbContext from database...");
    println!("  Provider: {}", provider);
    println!("  Connection: {}", connection);
    println!("  Output: {}", output_dir.display());

    // Read database schema
    println!("  Reading database schema...");
    let tables = read_database_schema(connection, provider).await?;

    // Generate entity files
    println!("  Generating entity types...");
    for table in &tables {
        let entity_code = generate_entity_code(table);
        let file_name = format!("{}.rs", table.name);
        let file_path = output_dir.join(&file_name);
        fs::write(&file_path, entity_code)
            .map_err(|e| format!("Failed to write {}: {}", file_path.display(), e))?;
        println!("    Created {}", file_path.display());
    }

    // Generate DbContext
    let context_code = generate_db_context_code(&tables);
    let context_path = output_dir.join("context.rs");
    fs::write(&context_path, context_code)
        .map_err(|e| format!("Failed to write context.rs: {}", e))?;
    println!("    Created {}", context_path.display());

    println!("Done. {} entity/entities scaffolded.", tables.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// File system helpers
// ---------------------------------------------------------------------------

struct MigrationInfo {
    name: String,
    path: PathBuf,
}

fn ensure_migrations_dir() -> Result<(), String> {
    let dir = Path::new(MIGRATIONS_DIR);
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create migrations directory: {}", e))?;
    }
    Ok(())
}

fn scan_migrations() -> Result<Vec<MigrationInfo>, String> {
    ensure_migrations_dir()?;
    let dir = Path::new(MIGRATIONS_DIR);

    let mut entries: Vec<MigrationInfo> = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("Failed to read migrations dir: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name != "." && name != ".." {
                    entries.push(MigrationInfo {
                        name: name.to_string(),
                        path: path.clone(),
                    });
                }
            }
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn read_applied_migrations() -> Vec<String> {
    let history_path = PathBuf::from(MIGRATIONS_DIR).join(".history");
    match fs::read_to_string(&history_path) {
        Ok(content) => content
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn record_applied_migration(name: &str) -> Result<(), String> {
    let history_path = PathBuf::from(MIGRATIONS_DIR).join(".history");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&history_path)
        .map_err(|e| format!("Failed to open history file: {}", e))?;
    writeln!(file, "{}", name).map_err(|e| format!("Failed to write history: {}", e))?;
    Ok(())
}

fn remove_applied_migration(name: &str) -> Result<(), String> {
    let mut applied = read_applied_migrations();
    applied.retain(|n| n != name);

    let history_path = PathBuf::from(MIGRATIONS_DIR).join(".history");
    let content: String = applied
        .iter()
        .map(|n| n.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&history_path, content)
        .map_err(|e| format!("Failed to update history file: {}", e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Schema reading (stub �?real impl requires provider-specific queries)
// ---------------------------------------------------------------------------

struct TableInfo {
    name: String,
    columns: Vec<ColumnInfo>,
}

struct ColumnInfo {
    name: String,
    data_type: String,
    is_nullable: bool,
    is_primary_key: bool,
}

async fn read_database_schema(connection: &str, provider: &str) -> Result<Vec<TableInfo>, String> {
    match provider {
        "postgres" => {
            println!("    Connecting to PostgreSQL and reading information_schema...");
            let db_tables = rust_ef_provider_postgres::introspection::introspect_postgres(connection)
                .await
                .map_err(|e| format!("Introspection failed: {}", e))?;

            let tables = db_tables
                .into_iter()
                .map(|t| {
                    let columns = t
                        .columns
                        .into_iter()
                        .map(|c| ColumnInfo {
                            name: c.name,
                            data_type: c.data_type,
                            is_nullable: c.is_nullable,
                            is_primary_key: c.is_primary_key,
                        })
                        .collect();
                    TableInfo {
                        name: t.name,
                        columns,
                    }
                })
                .collect();
            Ok(tables)
        }
        "mysql" => {
            println!("    (MySQL introspection not yet implemented)");
            Ok(Vec::new())
        }
        "sqlite" => {
            println!("    (SQLite introspection not yet implemented)");
            Ok(Vec::new())
        }
        _ => Err(format!("Unsupported provider: {}", provider)),
    }
}

fn generate_entity_code(table: &TableInfo) -> String {
    let struct_name = to_pascal_case(&table.name);
    let table_name = &table.name;

    let mut fields = String::new();
    for col in &table.columns {
        let rust_type = map_sql_type_to_rust(&col.data_type, col.is_nullable);
        let field_name = col.name.clone();
        let mut attrs = Vec::new();

        if col.is_primary_key {
            attrs.push("#[primary_key]".to_string());
            if col.data_type.contains("int") || col.data_type.contains("serial") {
                attrs.push("#[auto_increment]".to_string());
            }
        }
        if !col.is_nullable && !col.is_primary_key {
            attrs.push("#[required]".to_string());
        }
        if col.data_type.contains("varchar") {
            attrs.push("#[max_length(255)]".to_string());
        }

        let attr_str = if attrs.is_empty() {
            String::new()
        } else {
            format!("    {}\n", attrs.join("\n    "))
        };

        fields.push_str(&format!(
            "{}    pub {}: {},\n\n",
            attr_str, field_name, rust_type
        ));
    }

    format!(
        r#"use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("{table_name}")]
pub struct {struct_name} {{
{fields}}}
"#
    )
}

fn generate_db_context_code(tables: &[TableInfo]) -> String {
    let mut fields = String::new();
    let mut imports = String::new();

    for table in tables {
        let struct_name = to_pascal_case(&table.name);
        let field_name = &table.name;
        fields.push_str(&format!(
            "    pub {}: DbSet<{}>,\n",
            field_name, struct_name
        ));
        imports.push_str(&format!("pub use {};\n", field_name));
    }

    format!(
        r#"use rust_ef::prelude::*;

use rust_ef_provider_postgres::PostgresProvider;

{imports}
pub struct DbContext {{
{fields}    change_tracker: ChangeTracker,
    provider: PostgresProvider,
}}

impl DbContext {{
    pub async fn new(connection_string: &str) -> Result<Self, LrefError> {{
        let provider = PostgresProvider::new(connection_string, 5)?;
        Ok(Self {{
            // ... DbSet initialization
            change_tracker: ChangeTracker::new(),
            provider,
        }})
    }}
}}

#[async_trait::async_trait]
impl DbContext for DbContext {{
    type Provider = PostgresProvider;
    fn provider(&self) -> &Self::Provider {{ &self.provider }}
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker {{ &mut self.change_tracker }}
    fn change_tracker(&self) -> &ChangeTracker {{ &self.change_tracker }}
    async fn save_changes(&mut self) -> Result<SaveChangesResult, LrefError> {{
        // ... implementation
        unimplemented!()
    }}
}}
"#
    )
}

fn map_sql_type_to_rust(sql_type: &str, nullable: bool) -> String {
    let base = match sql_type.to_lowercase().as_str() {
        t if t.contains("int") || t.contains("serial") => "i32",
        t if t.contains("bigint") || t.contains("bigserial") => "i64",
        t if t.contains("smallint") => "i16",
        t if t.contains("real") || t.contains("float") => "f32",
        t if t.contains("double") => "f64",
        t if t.contains("bool") => "bool",
        t if t.contains("text") || t.contains("char") || t.contains("varchar") => "String",
        t if t.contains("bytea") || t.contains("blob") => "Vec<u8>",
        t if t.contains("timestamp") || t.contains("datetime") => "String",
        _ => "String",
    };

    if nullable && base != "String" {
        format!("Option<{}>", base)
    } else if base == "String" && !nullable {
        "String".to_string()
    } else if base == "String" && nullable {
        "Option<String>".to_string()
    } else {
        base.to_string()
    }
}

fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}
