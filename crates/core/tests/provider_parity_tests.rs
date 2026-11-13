//! EF-14: real-database provider-parity tests for MySQL and PostgreSQL.
//!
//! These tests need a running server, so they are `#[ignore]` by default. CI
//! runs them in the `provider-smoke` job of `.github/workflows/ci.yml`
//! (`services: mysql/postgres`). Locally:
//!
//! ```bash
//! export RUST_EF_MYSQL_URL='mysql://root:test@127.0.0.1:3306/rust_ef_test'
//! export RUST_EF_PG_URL='postgres://test:test@127.0.0.1:5432/rust_ef_test'
//! cargo test -p rust-ef --features chrono,uuid,decimal \
//!     --test provider_parity_tests -- --ignored --test-threads=1
//! ```
//!
//! SQLite parity needs no server and is covered by
//! `crates/sqlite/tests/introspection_tests.rs`.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::prelude::*;
use rust_ef::schema::DbTable;
use rust_ef_mysql::DbContextOptionsBuilderExt as MySqlExt;
use rust_ef_postgres::DbContextOptionsBuilderExt as PostgresExt;

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

fn require_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must point at an empty test database; see the test module docs")
    })
}

/// Asserts the introspected schema matches the model for every dialect.
fn assert_model_schema(tables: &[DbTable], dialect: MigrationDialect) {
    assert!(
        tables.iter().all(|table| !table.name.starts_with("__ef_")),
        "migration-internal tables must be excluded"
    );
    let products = tables
        .iter()
        .find(|table| table.name == "products")
        .expect("products table must be introspected");
    let id = products.column("id").expect("products.id");
    assert!(id.is_primary_key, "products.id must be a primary key");
    assert!(
        id.is_auto_increment,
        "{dialect:?}: products.id must be auto-generated, got {id:?}"
    );
    let sku = products.column("sku").expect("products.sku");
    assert_eq!(sku.max_length, Some(60), "{dialect:?}: sku max_length");
    assert!(!sku.is_nullable, "{dialect:?}: sku must be NOT NULL");
    assert!(products.column("price").is_some());

    assert!(
        products
            .indexes
            .iter()
            .any(|index| index.is_unique && index.column_names() == vec!["sku"]),
        "{dialect:?}: unique index on sku missing: {:?}",
        products.indexes
    );
    assert!(
        products
            .indexes
            .iter()
            .any(|index| !index.is_unique && index.column_names() == vec!["category_id"]),
        "{dialect:?}: index on category_id missing: {:?}",
        products.indexes
    );

    let foreign_key = products
        .foreign_keys
        .iter()
        .find(|fk| fk.referenced_table == "categories")
        .unwrap_or_else(|| {
            panic!(
                "{dialect:?}: FK products.category_id -> categories missing: {:?}",
                products.foreign_keys
            )
        });
    assert_eq!(foreign_key.columns, vec!["category_id"]);
    assert_eq!(foreign_key.referenced_columns, vec!["id"]);

    assert!(
        products.checks.is_some(),
        "{dialect:?}: CHECK constraints must be introspected"
    );
}

#[tokio::test]
#[ignore = "requires a MySQL server (RUST_EF_MYSQL_URL); run with --ignored"]
async fn mysql_introspection_matches_the_migrated_model() {
    let url = require_env("RUST_EF_MYSQL_URL");
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_mysql_with(&url, |options| {
        options.insecure();
    });
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("context");
    let metas = ctx.model().build();
    let engine = MigrationEngine::new(MigrationDialect::MySql);
    let migration = engine
        .generate("init", &metas, &None)
        .expect("generate migration");

    engine
        .apply(ctx.provider(), &migration)
        .await
        .expect("apply migration");
    // Installer restart: a second apply is a no-op (history table).
    engine
        .apply(ctx.provider(), &migration)
        .await
        .expect("re-apply must be idempotent");

    let tables = ctx
        .provider()
        .introspect()
        .await
        .expect("introspect MySQL schema");
    assert_model_schema(&tables, MigrationDialect::MySql);

    let products = tables
        .iter()
        .find(|table| table.name == "products")
        .expect("products table");
    // MySQL cannot expose re-executable trigger SQL through information_schema.
    assert_eq!(products.triggers, None);
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server (RUST_EF_PG_URL); run with --ignored"]
async fn postgres_introspection_matches_the_migrated_model() {
    let url = require_env("RUST_EF_PG_URL");
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_postgres_with(&url, |options| {
        options.insecure();
    });
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("context");
    let metas = ctx.model().build();
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let migration = engine
        .generate("init", &metas, &None)
        .expect("generate migration");

    engine
        .apply(ctx.provider(), &migration)
        .await
        .expect("apply migration");
    engine
        .apply(ctx.provider(), &migration)
        .await
        .expect("re-apply must be idempotent");

    let tables = ctx
        .provider()
        .introspect()
        .await
        .expect("introspect PostgreSQL schema");
    assert_model_schema(&tables, MigrationDialect::Postgres);

    let products = tables
        .iter()
        .find(|table| table.name == "products")
        .expect("products table");
    assert!(
        products.triggers.is_some(),
        "PostgreSQL introspects triggers"
    );
}
