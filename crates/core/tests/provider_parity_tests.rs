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
use rust_ef::error::EFError;
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::prelude::*;
use rust_ef::schema::DbTable;
use rust_ef_mysql::DbContextOptionsBuilderExt as MySqlExt;
use rust_ef_postgres::DbContextOptionsBuilderExt as PostgresExt;

#[derive(Debug, Clone, EntityType)]
#[table("parity_categories")]
struct Category {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(80)]
    #[unique]
    pub slug: String,
}

/// Custom enum used to exercise the readable/text-mapped enum path:
/// MySQL must render it as `VARCHAR(32)` so the `#[index]` below can be built
/// (unbounded `TEXT` fails with error 1170), PostgreSQL keeps `TEXT`.
#[derive(Debug, Clone, Default, PartialEq, Eq, DbEnum)]
enum ProductStatus {
    #[default]
    Active,
    Retired,
}

#[derive(Debug, Clone, EntityType)]
#[table("parity_products")]
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

    #[required]
    #[max_length(32)]
    #[index]
    pub status: ProductStatus,
}

/// Declarative indexes/CHECKs that reproduce the upstream defects on a real
/// server: a mixed column + expression index (MySQL 3762), and a CHECK whose
/// `IN (...)` list PostgreSQL canonicalizes to `= ANY (ARRAY[...])`.
#[derive(Default)]
struct ProductConfig;

#[entity(Product)]
impl IEntityTypeConfiguration<Product> for ProductConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Product>) {
        entity
            .has_index_expression(&["sku", "coalesce(price, 0)"])
            .has_name("ix_products_sku_price_expr");
        entity.has_check("ck_products_sku_choice", "sku IN ('basic','pro')");
    }
}

/// EF-DDL-01: a composite auto-name longer than 63 bytes must still create on
/// both MySQL (limit 64) and PostgreSQL (silent truncation at 63).
#[derive(Debug, Clone, EntityType)]
#[table("ef_long_identifier_acceptance_table_name_for_release")]
struct LongName {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(32)]
    pub ef_long_column_name_number_one: String,

    #[required]
    #[max_length(32)]
    pub ef_long_column_name_number_two: String,

    #[required]
    #[max_length(32)]
    pub ef_long_column_name_number_three: String,

    #[required]
    #[max_length(32)]
    pub ef_long_column_name_number_four: String,
}

#[derive(Default)]
struct LongNameConfig;

#[entity(LongName)]
impl IEntityTypeConfiguration<LongName> for LongNameConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, LongName>) {
        entity.has_index(&[
            LongName::COLUMN_EF_LONG_COLUMN_NAME_NUMBER_ONE,
            LongName::COLUMN_EF_LONG_COLUMN_NAME_NUMBER_TWO,
            LongName::COLUMN_EF_LONG_COLUMN_NAME_NUMBER_THREE,
            LongName::COLUMN_EF_LONG_COLUMN_NAME_NUMBER_FOUR,
        ]);
    }
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
        .find(|table| table.name == "parity_products")
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
        .find(|fk| fk.referenced_table == "parity_categories")
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

    let status = products.column("status").expect("products.status");
    match dialect {
        // EF-MYSQL-01: the custom enum must be a bounded VARCHAR, not TEXT,
        // otherwise the index below cannot be created.
        MigrationDialect::MySql => {
            assert_eq!(
                status.data_type.to_ascii_lowercase(),
                "varchar(32)",
                "MySQL enum column must be VARCHAR(32)"
            );
            assert_eq!(status.max_length, Some(32));
        }
        MigrationDialect::Postgres => {
            assert_eq!(
                status.data_type.to_ascii_lowercase(),
                "text",
                "PostgreSQL keeps TEXT for custom enums"
            );
        }
        MigrationDialect::Sqlite => {}
    }
    assert!(
        products
            .indexes
            .iter()
            .any(|index| index.column_names() == vec!["status"]),
        "{dialect:?}: index on the enum column missing: {:?}",
        products.indexes
    );

    // EF-MYSQL-02: the mixed `(sku, coalesce(price, 0))` index must exist and
    // its plain column must not be wrapped as an expression.
    let expression_index = products
        .indexes
        .iter()
        .find(|index| index.name == "ix_products_sku_price_expr")
        .unwrap_or_else(|| {
            panic!(
                "{dialect:?}: mixed expression index missing: {:?}",
                products.indexes
            )
        });
    assert!(
        expression_index
            .columns
            .iter()
            .any(|part| !part.is_expression && part.value.eq_ignore_ascii_case("sku")),
        "{dialect:?}: mixed index must keep `sku` as a plain column: {:?}",
        expression_index.columns
    );
    assert!(
        expression_index.columns.iter().any(|part| {
            part.is_expression && part.value.to_ascii_lowercase().contains("coalesce")
        }),
        "{dialect:?}: mixed index must keep the expression part: {:?}",
        expression_index.columns
    );

    // EF-DDL-01: the long auto-derived name must be a valid identifier on both
    // servers (no MySQL 1059, no PostgreSQL truncation surprise).
    let long_table = tables
        .iter()
        .find(|table| table.name == "ef_long_identifier_acceptance_table_name_for_release")
        .expect("long-name table must be introspected");
    let long_index = long_table
        .indexes
        .iter()
        .find(|index| index.columns.len() == 4)
        .unwrap_or_else(|| {
            panic!(
                "{dialect:?}: long-name composite index missing: {:?}",
                long_table.indexes
            )
        });
    assert!(
        long_index.name.len() <= 63,
        "{dialect:?}: auto index name must be <= 63 bytes, got {} ({})",
        long_index.name.len(),
        long_index.name
    );
}

/// EF-DIFF-01 acceptance: a schema created from the model must introspect and
/// diff back to **zero statements** on the same connection.
async fn assert_no_runtime_drift(
    engine: &MigrationEngine,
    metas: &[rust_ef::metadata::EntityTypeMeta],
    tables: &[DbTable],
    dialect: MigrationDialect,
) {
    let diff = engine.diff_metas(metas, tables);
    assert!(
        diff.statements.is_empty(),
        "{dialect:?}: fresh schema must diff to no statements:\n{}",
        diff.report()
    );
    assert!(
        !diff.report().to_ascii_uppercase().contains("DROP CHECK"),
        "{dialect:?}: runtime diff must never drop a CHECK:\n{}",
        diff.report()
    );
    assert!(
        !diff
            .report()
            .to_ascii_uppercase()
            .contains("DROP CONSTRAINT"),
        "{dialect:?}: runtime diff must never drop a constraint:\n{}",
        diff.report()
    );
}

/// EF-ERR-01 acceptance: a failed statement keeps SQLSTATE, driver message and
/// the SQL in `EFError::Database`.
async fn assert_structured_driver_error(provider: &dyn IDatabaseProvider, sql: &str) {
    let mut connection = provider.get_connection().await.expect("connection");
    let error = connection
        .execute(sql, &[])
        .await
        .expect_err("a missing table must fail");
    match error {
        EFError::Database(info, _) => {
            assert!(
                info.sqlstate.is_some(),
                "driver error must keep SQLSTATE: {info:?}"
            );
            assert!(
                info.message
                    .to_ascii_lowercase()
                    .contains("__ef_missing_2_5__"),
                "driver message must name the missing object: {info:?}"
            );
            assert_eq!(
                info.sql.as_deref(),
                Some(sql),
                "driver error must keep the failing SQL"
            );
        }
        other => panic!("expected EFError::Database, got {other:?}"),
    }
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
        .generate("provider_parity_init", &metas, &None)
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
    assert_no_runtime_drift(&engine, &metas, &tables, MigrationDialect::MySql).await;
    assert_structured_driver_error(ctx.provider(), "DELETE FROM __ef_missing_2_5__").await;

    let products = tables
        .iter()
        .find(|table| table.name == "parity_products")
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
        .generate("provider_parity_init", &metas, &None)
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
    assert_no_runtime_drift(&engine, &metas, &tables, MigrationDialect::Postgres).await;
    assert_structured_driver_error(ctx.provider(), "DELETE FROM __ef_missing_2_5__").await;

    let products = tables
        .iter()
        .find(|table| table.name == "parity_products")
        .expect("products table");
    assert!(
        products.triggers.is_some(),
        "PostgreSQL introspects triggers"
    );
}

// EF-PG-01: the `DbValue::Null` binding matrix against a real PostgreSQL server
// lives in `tests/postgres_null_binding.rs`
// (`RUST_EF_PG_URL=... cargo test -p rust-ef --test postgres_null_binding -- --ignored`).
