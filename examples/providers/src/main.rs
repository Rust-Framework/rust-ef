//! Provider-parity example: **probe → generate → apply → introspect** on
//! SQLite, MySQL and PostgreSQL with the same code path.
//!
//! This is the reference "installer + prober" sequence: a clean database is
//! migrated from the model, then the live schema is read back through
//! `IDatabaseProvider::introspect()` and compared with what the model declared.
//!
//! ## Run it
//!
//! ```text
//! # SQLite (file is recreated on every run)
//! cargo run -p rust-ef-providers-example -- --provider sqlite --url /tmp/rust_ef_example.db
//!
//! # MySQL / PostgreSQL (use any empty database; CI provides a service)
//! RUST_EF_MYSQL_URL=mysql://root:test@127.0.0.1:3306/rust_ef_test \
//!   cargo run -p rust-ef-providers-example -- --provider mysql --url "$RUST_EF_MYSQL_URL"
//! RUST_EF_PG_URL=postgres://test:test@127.0.0.1:5432/rust_ef_test \
//!   cargo run -p rust-ef-providers-example -- --provider postgres --url "$RUST_EF_PG_URL"
//! ```
//!
//! `--provider mysql|postgres` can omit `--url` when the corresponding
//! environment variable is set.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::prelude::*;
use rust_ef_mysql::DbContextOptionsBuilderExt as MySqlExt;
use rust_ef_postgres::DbContextOptionsBuilderExt as PostgresExt;
use rust_ef_sqlite::DbContextOptionsBuilderExt as SqliteExt;

#[derive(Debug, Clone, EntityType)]
#[table("categories")]
struct Category {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(80)]
    #[unique]
    pub slug: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("products")]
struct Product {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(60)]
    #[unique]
    pub sku: String,

    #[foreign_key(Category)]
    #[index]
    pub category_id: i32,

    pub price: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    Sqlite,
    MySql,
    Postgres,
}

impl ProviderKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            "mysql" => Ok(Self::MySql),
            "postgres" | "postgresql" | "pg" => Ok(Self::Postgres),
            other => Err(format!(
                "unknown provider '{other}' (expected sqlite|mysql|postgres)"
            )),
        }
    }

    fn dialect(self) -> MigrationDialect {
        match self {
            Self::Sqlite => MigrationDialect::Sqlite,
            Self::MySql => MigrationDialect::MySql,
            Self::Postgres => MigrationDialect::Postgres,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::MySql => "mysql",
            Self::Postgres => "postgres",
        }
    }
}

fn parse_args() -> Result<(ProviderKind, String), String> {
    let mut args = std::env::args().skip(1);
    let mut provider = None;
    let mut url = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--provider" => {
                let value = args.next().ok_or("--provider needs a value")?;
                provider = Some(ProviderKind::parse(&value)?);
            }
            "--url" => url = Some(args.next().ok_or("--url needs a value")?),
            "--help" | "-h" => {
                return Err("usage: rust-ef-providers-example --provider <sqlite|mysql|postgres> [--url <connection-string>]".into());
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let provider = provider.ok_or("--provider is required")?;
    let url = match url {
        Some(url) => url,
        None => match provider {
            ProviderKind::Sqlite => {
                return Err("--provider sqlite requires --url <path>".into());
            }
            ProviderKind::MySql => std::env::var("RUST_EF_MYSQL_URL")
                .map_err(|_| "--provider mysql requires --url or RUST_EF_MYSQL_URL".to_string())?,
            ProviderKind::Postgres => std::env::var("RUST_EF_PG_URL")
                .map_err(|_| "--provider postgres requires --url or RUST_EF_PG_URL".to_string())?,
        },
    };
    Ok((provider, url))
}

#[tokio::main]
async fn main() -> Result<(), EFError> {
    let (provider_kind, url) = match parse_args() {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };

    println!(
        "=== rust-ef provider example: {} ===",
        provider_kind.label()
    );
    println!("url: {url}");

    // A clean SQLite file makes every run start from an empty database; MySQL
    // and PostgreSQL databases are expected to be empty (CI provides one per
    // job) and the migration history makes reruns idempotent.
    if provider_kind == ProviderKind::Sqlite {
        let _ = std::fs::remove_file(&url);
    }

    let mut builder = DbContextOptionsBuilder::new();
    match provider_kind {
        ProviderKind::Sqlite => {
            builder.use_sqlite(&url);
        }
        ProviderKind::MySql => {
            // CI/local services run plaintext; production uses TLS by default.
            builder.use_mysql_with(&url, |options| {
                options.insecure();
            });
        }
        ProviderKind::Postgres => {
            builder.use_postgres_with(&url, |options| {
                options.insecure();
            });
        }
    }
    let options = builder.build();

    // [1] Model → migration SQL (generate).
    let mut ctx = DbContext::from_options(&options)?;
    let metas = ctx.model().build();
    let engine = MigrationEngine::new(provider_kind.dialect());
    let migration = engine.generate("init", &metas, &None)?;
    println!(
        "[1/4] generated migration '{}' ({} bytes of Up SQL)",
        migration.id,
        migration.up_sql.len()
    );

    // [2] Probe: the provider must be reachable before applying anything.
    {
        let mut conn = ctx.provider().get_connection().await?;
        let rows = conn.query("SELECT 1", &[]).await?;
        assert_eq!(rows.len(), 1, "probe query must return one row");
        println!("[2/4] probe OK (SELECT 1)");
    }

    // [3] Apply, then re-apply to prove the history table makes it idempotent
    // (this is the "restart the installer" path).
    engine.apply(ctx.provider(), &migration).await?;
    engine.apply(ctx.provider(), &migration).await?;
    println!("[3/4] apply OK (idempotent on re-run)");

    // [4] Introspect the live database through the unified provider entry point.
    let tables = ctx.provider().introspect().await?;
    let products = tables
        .iter()
        .find(|table| table.name == "products")
        .expect("products table must be introspected");
    assert!(
        tables.iter().any(|table| table.name == "categories"),
        "categories table must be introspected"
    );

    let id = products
        .column("id")
        .expect("products.id must be introspected");
    assert!(id.is_primary_key, "products.id must be the primary key");
    assert!(
        id.is_auto_increment,
        "products.id must be auto-generated on {}",
        provider_kind.label()
    );
    let sku = products
        .column("sku")
        .expect("products.sku must be introspected");
    assert_eq!(sku.max_length, Some(60));
    assert!(!sku.is_nullable);

    assert!(
        products
            .indexes
            .iter()
            .any(|index| index.is_unique && index.column_names() == vec!["sku"]),
        "unique index on sku missing: {:?}",
        products.indexes
    );
    assert!(
        products
            .indexes
            .iter()
            .any(|index| !index.is_unique && index.column_names() == vec!["category_id"]),
        "index on category_id missing: {:?}",
        products.indexes
    );

    let foreign_key = products
        .foreign_keys
        .iter()
        .find(|fk| fk.referenced_table == "categories")
        .unwrap_or_else(|| {
            panic!(
                "foreign key products.category_id -> categories missing: {:?}",
                products.foreign_keys
            )
        });
    assert_eq!(foreign_key.columns, vec!["category_id"]);
    assert_eq!(foreign_key.referenced_columns, vec!["id"]);

    // Provider capability differences are exposed, not hidden:
    // - SQLite introspects checks *and* triggers;
    // - PostgreSQL introspects checks *and* triggers;
    // - MySQL introspects checks on 8.0.16+ but not full trigger SQL (`None`).
    assert!(
        products.checks.is_some(),
        "{} must introspect CHECK constraints",
        provider_kind.label()
    );
    match provider_kind {
        ProviderKind::Sqlite | ProviderKind::Postgres => assert!(
            products.triggers.is_some(),
            "{} must introspect triggers",
            provider_kind.label()
        ),
        ProviderKind::MySql => assert_eq!(
            products.triggers, None,
            "MySQL does not expose re-executable trigger SQL"
        ),
    }

    println!(
        "[4/4] introspect OK: {} tables; products = {} columns, {} indexes, {} foreign keys, checks={:?}, triggers={:?}",
        tables.len(),
        products.columns.len(),
        products.indexes.len(),
        products.foreign_keys.len(),
        products.checks.as_ref().map(Vec::len),
        products.triggers.as_ref().map(Vec::len),
    );
    println!(
        "OK: generate -> apply -> introspect passed on {}",
        provider_kind.label()
    );
    Ok(())
}
