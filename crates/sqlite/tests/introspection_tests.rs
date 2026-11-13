//! EF-5: SQLite live-schema introspection.
//!
//! The acceptance criterion is "introspect a migrated model and get an
//! equivalent `DbTable`": tables, columns (PK / nullable / max length /
//! auto-increment), indexes (unique / plain), and foreign keys.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::prelude::*;
use rust_ef::provider::IDatabaseProvider;
use rust_ef_sqlite::{DbContextOptionsBuilderExt, SqliteProvider};

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

async fn migrated_provider() -> (
    rust_ef::db_context::DbContextOptions,
    std::sync::Arc<dyn IDatabaseProvider>,
) {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).expect("context");
    let metas = ctx.model().build();
    assert!(
        metas.iter().any(|meta| meta.table_name == "products"),
        "Product must be discovered"
    );

    let provider = options.create_provider().expect("provider");
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let migration = engine
        .generate("init", &metas, &None)
        .expect("generate migration");
    engine
        .apply(&*provider, &migration)
        .await
        .expect("apply migration");
    (options, provider)
}

#[tokio::test]
async fn sqlite_introspection_matches_the_migrated_model() {
    let (_options, provider) = migrated_provider().await;
    let tables = provider.introspect().await.expect("introspect");

    assert!(
        tables.iter().all(|table| !table.name.starts_with("__ef_")),
        "migration-internal tables must be excluded: {:?}",
        tables.iter().map(|t| &t.name).collect::<Vec<_>>()
    );

    let products = tables
        .iter()
        .find(|table| table.name == "products")
        .expect("products table");

    // Columns.
    let id = products.column("id").expect("id column");
    assert!(id.is_primary_key);
    assert!(id.is_auto_increment, "INTEGER PRIMARY KEY is a rowid alias");
    assert!(id.is_not_null());
    let sku = products.column("sku").expect("sku column");
    assert!(!sku.is_nullable);
    assert_eq!(sku.max_length, Some(60));
    assert!(products.column("price").is_some());

    // Indexes: one unique (from #[unique]) and one plain (from #[index]).
    assert!(
        products
            .indexes
            .iter()
            .any(|index| index.is_unique && index.column_names() == vec!["sku"]),
        "expected a unique index on sku, got {:?}",
        products.indexes
    );
    assert!(
        products
            .indexes
            .iter()
            .any(|index| !index.is_unique && index.column_names() == vec!["category_id"]),
        "expected a plain index on category_id, got {:?}",
        products.indexes
    );

    // Foreign key (SQLite does not expose constraint names).
    let foreign_key = products
        .foreign_keys
        .iter()
        .find(|fk| fk.referenced_table == "categories")
        .expect("foreign key to categories");
    assert_eq!(foreign_key.columns, vec!["category_id"]);
    assert_eq!(foreign_key.referenced_columns, vec!["id"]);
    assert_eq!(foreign_key.name, None);
    assert_eq!(foreign_key.on_delete, None);

    // SQLite can introspect checks/triggers, so `Some(empty)` means "confirmed
    // none" (the model declares none).
    assert_eq!(products.checks, Some(Vec::new()));
    assert_eq!(products.triggers, Some(Vec::new()));
}

#[tokio::test]
async fn dbcontext_provider_exposes_introspection() {
    // This is the Travel installer/prober entry point: the provider reachable
    // from a `DbContext` must support the same unified introspection call.
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let ctx = DbContext::from_options(&options).expect("context");
    ctx.ensure_created().await.expect("ensure_created");

    let tables = ctx
        .provider()
        .introspect()
        .await
        .expect("introspect via DbContext provider");
    assert!(tables.iter().any(|table| table.name == "products"));
    assert!(tables.iter().any(|table| table.name == "categories"));
}

#[tokio::test]
async fn sqlite_introspects_checks_triggers_partial_and_expression_indexes() {
    let provider = SqliteProvider::new_in_memory().expect("provider");
    provider
        .execute_migration_command(
            "CREATE TABLE items (\
                 id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 name TEXT NOT NULL, \
                 status TEXT NOT NULL, \
                 price INTEGER NOT NULL CHECK (price > 0)\
             ); \
             CREATE UNIQUE INDEX ux_items_name ON items (lower(name)) WHERE status = 'PENDING'; \
             CREATE TRIGGER trg_items_ai AFTER INSERT ON items \
                 BEGIN UPDATE items SET status = 'PENDING' WHERE id = NEW.id; END;",
        )
        .await
        .expect("create schema");

    let tables = provider.introspect().await.expect("introspect");
    let items = tables
        .iter()
        .find(|table| table.name == "items")
        .expect("items table");

    let checks = items.checks.as_ref().expect("checks introspected");
    assert_eq!(checks.len(), 1, "got {checks:?}");
    assert_eq!(checks[0].name, None);
    assert!(
        checks[0].expression.contains("price > 0"),
        "got {}",
        checks[0].expression
    );

    let index = items
        .indexes
        .iter()
        .find(|index| index.name == "ux_items_name")
        .expect("expression index");
    assert!(index.is_unique);
    assert_eq!(index.filter.as_deref(), Some("status = 'PENDING'"));
    assert_eq!(index.columns.len(), 1);
    assert!(index.columns[0].is_expression);
    assert_eq!(index.columns[0].value, "lower(name)");
    assert!(index.column_names().is_empty());

    let triggers = items.triggers.as_ref().expect("triggers introspected");
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].name, "trg_items_ai");
    assert!(triggers[0].sql.contains("AFTER INSERT"));
}
