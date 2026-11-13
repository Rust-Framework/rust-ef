//! EF Core `EntityTypeBuilder` / `PropertyBuilder` parity coverage:
//! column type/precision/fixed length/collation/comments, computed columns,
//! alternate keys, schema-qualified tables, views/keyless entities, and
//! index INCLUDE.

use rust_ef::entity::IEntityType;
use rust_ef::metadata::EntityTypeMeta;
use rust_ef::migration::{MigrationDialect, MigrationEngine, MigrationStore};
use rust_ef::model_builder::{EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder};
use rust_ef::provider::{DbValue, IDatabaseProvider};
use rust_ef::{linq, EntityType};

#[derive(Debug, Clone, EntityType)]
#[table("products", schema = "shop", comment = "Product catalog")]
struct Product {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[column_type("VARCHAR(36)")]
    pub sku: String,
    #[fixed_length]
    #[max_length(3)]
    pub currency: String,
    #[collation("utf8mb4_bin")]
    pub code: String,
    #[comment("unit price in minor units")]
    pub unit_price: i64,
    #[precision(14, 4)]
    pub weight: i64,
    pub quantity: i32,
    #[computed("unit_price * quantity", stored)]
    pub total: i64,
}

#[derive(Default)]
struct ProductConfig;

#[rust_ef::entity(Product)]
impl IEntityTypeConfiguration<Product> for ProductConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Product>) {
        // Alternate key (unique constraint) on the business code.
        entity.has_alternate_key(&[Product::COLUMN_CODE]);
        // Index with PostgreSQL-style INCLUDE payload.
        entity
            .has_index(&[Product::COLUMN_CODE])
            .include(&[Product::COLUMN_QUANTITY]);
    }
}

#[derive(Debug, Clone, EntityType)]
#[table("active_products", view)]
struct ActiveProduct {
    pub id: i32,
    pub sku: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("product_stats")]
#[keyless]
struct ProductStat {
    pub quantity: i32,
}

#[derive(Debug, Clone, EntityType)]
#[table("line_items")]
struct LineItem {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub unit_price: i64,
    pub quantity: i32,
    #[computed("unit_price * quantity", stored)]
    pub total: i64,
}

#[derive(Debug, Clone, EntityType)]
#[table("legacy")]
struct Legacy {
    #[primary_key]
    pub id: i32,
    pub code: String,
    #[computed("id * 2", stored)]
    pub qty: i32,
    pub internal: String,
    pub price: i64,
}

#[derive(Default)]
struct LegacyConfig;

impl IEntityTypeConfiguration<Legacy> for LegacyConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Legacy>) {
        entity.to_schema("archive");
        entity.has_comment("legacy table");
        entity.ignore("internal");
        entity
            .property_named("code")
            .has_column_type("CHAR(8)")
            .has_collation("utf8mb4_bin")
            .has_comment("business code");
        entity
            .property_named("price")
            .has_precision(12, 2)
            .has_default("0");
        entity.has_alternate_key(&[Legacy::COLUMN_CODE]);
        entity
            .has_index(&[Legacy::COLUMN_QTY])
            .include(&[Legacy::COLUMN_CODE]);
    }
}

fn model() -> Vec<EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.apply_configuration::<ProductConfig, Product>();
    builder.apply_configuration::<LegacyConfig, Legacy>();
    builder.entity::<ActiveProduct>();
    builder.entity::<ProductStat>();
    builder.entity::<LineItem>();
    builder.build()
}

#[test]
fn fluent_api_sets_column_and_table_options() {
    let meta = model()
        .into_iter()
        .find(|m| m.table_name == "archive.legacy")
        .expect("legacy entity");

    assert_eq!(meta.comment.as_deref(), Some("legacy table"));

    let internal = meta.find_property("internal").unwrap();
    assert!(internal.is_not_mapped, "ignore() must exclude the field");

    let code = meta.find_property("code").unwrap();
    assert_eq!(code.column_type.as_deref(), Some("CHAR(8)"));
    assert_eq!(code.collation.as_deref(), Some("utf8mb4_bin"));
    assert_eq!(code.comment.as_deref(), Some("business code"));

    let price = meta.find_property("price").unwrap();
    assert_eq!(price.precision, Some(12));
    assert_eq!(price.scale, Some(2));
    assert_eq!(price.default_value.as_deref(), Some("0"));

    let qty = meta.find_property("qty").unwrap();
    assert_eq!(qty.computed_sql.as_deref(), Some("id * 2"));
    assert!(qty.computed_stored);

    assert!(meta
        .indexes
        .iter()
        .any(|i| i.name.as_deref() == Some("ak_archive_legacy_code")));
    assert!(meta
        .indexes
        .iter()
        .any(|i| { i.include.iter().map(|c| c.as_ref()).collect::<Vec<_>>() == vec!["code"] }));
}

#[test]
fn fluent_api_ddl_matches_derive_attributes() {
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let migration = engine.generate("Init", &model(), &None).unwrap();
    let sql = &migration.up_sql;

    assert!(
        sql.contains("CREATE TABLE \"archive\".\"legacy\""),
        "missing schema override:\n{sql}"
    );
    assert!(
        !sql.contains("\"internal\""),
        "ignored column must not appear in DDL:\n{sql}"
    );
    assert!(
        sql.contains("\"code\" CHAR(8) COLLATE utf8mb4_bin NOT NULL"),
        "missing fluent column type/collation:\n{sql}"
    );
    assert!(
        sql.contains("COMMENT ON COLUMN \"archive\".\"legacy\".\"code\" IS 'business code';"),
        "missing fluent column comment:\n{sql}"
    );
    assert!(
        sql.contains("COMMENT ON TABLE \"archive\".\"legacy\" IS 'legacy table';"),
        "missing fluent table comment:\n{sql}"
    );
}

#[test]
fn derive_attributes_flow_into_metadata() {
    let meta = Product::entity_meta();
    assert_eq!(meta.table_name, "shop.products");
    assert_eq!(meta.comment.as_deref(), Some("Product catalog"));

    let sku = meta.find_property("sku").unwrap();
    assert_eq!(sku.column_type.as_deref(), Some("VARCHAR(36)"));

    let currency = meta.find_property("currency").unwrap();
    assert!(currency.is_fixed_length);
    assert_eq!(currency.max_length, Some(3));

    let code = meta.find_property("code").unwrap();
    assert_eq!(code.collation.as_deref(), Some("utf8mb4_bin"));

    let unit_price = meta.find_property("unit_price").unwrap();
    assert_eq!(
        unit_price.comment.as_deref(),
        Some("unit price in minor units")
    );

    let weight = meta.find_property("weight").unwrap();
    assert_eq!(weight.precision, Some(14));
    assert_eq!(weight.scale, Some(4));

    let total = meta.find_property("total").unwrap();
    assert_eq!(total.computed_sql.as_deref(), Some("unit_price * quantity"));
    assert!(total.computed_stored);
    assert!(total.is_computed());

    assert!(ActiveProduct::entity_meta().is_view);
    assert!(ProductStat::entity_meta().is_keyless);
}

#[test]
fn computed_columns_are_excluded_from_writes() {
    let meta = Product::entity_meta();
    let writable: Vec<&str> = meta
        .writable_scalar_properties()
        .map(|p| p.column_name.as_ref())
        .collect();
    assert!(writable.contains(&"unit_price"));
    assert!(writable.contains(&"quantity"));
    assert!(
        !writable.contains(&"total"),
        "computed column must not be inserted/updated: {writable:?}"
    );
}

#[test]
fn postgres_ddl_covers_entity_configuration() {
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let migration = engine.generate("Init", &model(), &None).unwrap();
    let sql = &migration.up_sql;

    // Schema-qualified table.
    assert!(
        sql.contains("CREATE TABLE \"shop\".\"products\""),
        "missing schema-qualified table:\n{sql}"
    );
    // Column type override + fixed length + collation.
    assert!(
        sql.contains("\"sku\" VARCHAR(36)"),
        "missing column type override:\n{sql}"
    );
    assert!(
        sql.contains("\"currency\" CHAR(3)"),
        "missing fixed-length CHAR:\n{sql}"
    );
    assert!(
        sql.contains("\"code\" TEXT COLLATE utf8mb4_bin NOT NULL"),
        "missing column collation:\n{sql}"
    );
    // Computed column (PostgreSQL requires STORED).
    assert!(
        sql.contains("\"total\" BIGINT GENERATED ALWAYS AS (unit_price * quantity) STORED"),
        "missing generated column:\n{sql}"
    );
    // Table + column comments.
    assert!(
        sql.contains("COMMENT ON TABLE \"shop\".\"products\" IS 'Product catalog';"),
        "missing table comment:\n{sql}"
    );
    assert!(
        sql.contains(
            "COMMENT ON COLUMN \"shop\".\"products\".\"unit_price\" IS 'unit price in minor units';"
        ),
        "missing column comment:\n{sql}"
    );
    // Alternate key.
    assert!(
        sql.contains("CREATE UNIQUE INDEX \"ak_shop_products_code\""),
        "missing alternate key:\n{sql}"
    );
    // INCLUDE.
    assert!(
        sql.contains("INCLUDE (\"quantity\")"),
        "missing index INCLUDE:\n{sql}"
    );
    // View is not created.
    assert!(
        !sql.contains("CREATE TABLE \"active_products\""),
        "view must not create a table:\n{sql}"
    );
    // Keyless table has no PRIMARY KEY.
    assert!(
        sql.contains("CREATE TABLE \"product_stats\""),
        "keyless table missing:\n{sql}"
    );
    let stats_block = sql
        .split("CREATE TABLE \"product_stats\"")
        .nth(1)
        .and_then(|s| s.split(';').next())
        .unwrap();
    assert!(
        !stats_block.contains("PRIMARY KEY"),
        "keyless table must not emit PRIMARY KEY:\n{stats_block}"
    );
}

#[test]
fn mysql_ddl_uses_inline_comments_and_type_overrides() {
    let engine = MigrationEngine::new(MigrationDialect::MySql);
    let migration = engine.generate("Init", &model(), &None).unwrap();
    let sql = &migration.up_sql;

    assert!(
        sql.contains("CREATE TABLE `shop`.`products`"),
        "missing schema-qualified table:\n{sql}"
    );
    assert!(
        sql.contains("`sku` VARCHAR(36)"),
        "missing column type override:\n{sql}"
    );
    assert!(
        sql.contains("`currency` CHAR(3)"),
        "missing fixed-length column:\n{sql}"
    );
    assert!(
        sql.contains("`code` TEXT COLLATE utf8mb4_bin NOT NULL"),
        "missing collation:\n{sql}"
    );
    assert!(
        sql.contains("`total` BIGINT GENERATED ALWAYS AS (unit_price * quantity) STORED"),
        "missing generated column:\n{sql}"
    );
    assert!(
        sql.contains("COMMENT 'unit price in minor units'"),
        "missing inline column comment:\n{sql}"
    );
    assert!(
        sql.contains("COMMENT = 'Product catalog'"),
        "missing table comment:\n{sql}"
    );
    // MySQL does not support INCLUDE: warning, index still created.
    assert!(
        sql.contains("index INCLUDE is only supported by PostgreSQL"),
        "missing INCLUDE warning:\n{sql}"
    );
}

#[test]
fn registered_snapshot_helper_applies_inventory_configuration() {
    let snapshot =
        rust_ef::migration::build_registered_snapshot(None, MigrationDialect::Postgres).unwrap();
    let products = snapshot
        .entity_types
        .iter()
        .find(|et| et.table_name == "shop.products")
        .expect("inventory snapshot includes Product");
    assert!(
        products
            .indexes
            .iter()
            .any(|i| i.name == "ak_shop_products_code"),
        "inventory #[entity] configuration must be applied"
    );
    assert!(snapshot
        .entity_types
        .iter()
        .any(|et| et.table_name == "active_products" && et.is_view));
}

#[test]
fn snapshot_json_roundtrips_entity_configuration() {
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let snapshot = engine.create_snapshot("cfg", &model());

    let dir = std::env::temp_dir().join("rust_ef_entity_config_snapshot");
    let _ = std::fs::remove_dir_all(&dir);
    let store = MigrationStore::new(&dir);
    store.save_snapshot(&snapshot).unwrap();
    let loaded = store.load_snapshot().unwrap().expect("snapshot loads");

    let products = loaded
        .entity_types
        .iter()
        .find(|et| et.table_name == "shop.products")
        .expect("products in snapshot");
    let sku = products
        .columns
        .iter()
        .find(|c| c.column_name == "sku")
        .unwrap();
    assert_eq!(sku.column_type.as_deref(), Some("VARCHAR(36)"));
    let currency = products
        .columns
        .iter()
        .find(|c| c.column_name == "currency")
        .unwrap();
    assert!(currency.is_fixed_length);
    let total = products
        .columns
        .iter()
        .find(|c| c.column_name == "total")
        .unwrap();
    assert_eq!(total.computed_sql.as_deref(), Some("unit_price * quantity"));
    assert!(total.computed_stored);
    assert_eq!(products.comment.as_deref(), Some("Product catalog"));
    assert!(products
        .indexes
        .iter()
        .any(|i| i.include == vec!["quantity".to_string()]));
    assert!(loaded
        .entity_types
        .iter()
        .any(|et| et.table_name == "active_products" && et.is_view));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dialect_maps_precision_scale_and_fixed_length() {
    let decimal = rust_ef::migration::SnapshotColumn {
        type_name: "rust_decimal::Decimal".into(),
        precision: Some(12),
        scale: Some(2),
        ..Default::default()
    };
    assert_eq!(
        MigrationDialect::MySql.map_column_type(&decimal),
        "DECIMAL(12,2)"
    );
    assert_eq!(
        MigrationDialect::Postgres.map_column_type(&decimal),
        "NUMERIC(12,2)"
    );

    let fixed = rust_ef::migration::SnapshotColumn {
        type_name: "alloc::string::String".into(),
        max_length: Some(3),
        is_fixed_length: true,
        ..Default::default()
    };
    assert_eq!(MigrationDialect::MySql.map_column_type(&fixed), "CHAR(3)");

    let explicit = rust_ef::migration::SnapshotColumn {
        type_name: "alloc::string::String".into(),
        column_type: Some("JSON".into()),
        ..Default::default()
    };
    assert_eq!(MigrationDialect::MySql.map_column_type(&explicit), "JSON");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, rust_ef::DbEnum)]
enum OrderStatus {
    #[default]
    Pending,
    #[db_value("PAID")]
    Paid,
    Cancelled,
}

#[derive(Debug, Clone, EntityType)]
#[table("orders_enum")]
struct OrderEnum {
    #[primary_key]
    pub id: i32,
    #[column_type("VARCHAR(16)")]
    pub status: OrderStatus,
    pub archived_status: Option<OrderStatus>,
}

#[test]
fn db_enum_maps_to_string_values() {
    use rust_ef::entity::{IEntitySnapshot, IFromRow};

    assert_eq!(
        DbValue::from(OrderStatus::Paid),
        DbValue::String("PAID".into())
    );
    assert_eq!(
        OrderStatus::try_from(DbValue::String("Cancelled".into())).unwrap(),
        OrderStatus::Cancelled
    );
    assert!(OrderStatus::try_from(DbValue::String("NOPE".into())).is_err());
    assert_eq!(OrderStatus::Paid.as_str(), "PAID");
    assert_eq!(OrderStatus::from_db_str("PAID"), Some(OrderStatus::Paid));

    let row = OrderEnum::from_row(&[
        DbValue::I32(1),
        DbValue::String("PAID".into()),
        DbValue::Null,
    ])
    .unwrap();
    assert_eq!(row.status, OrderStatus::Paid);
    assert_eq!(row.archived_status, None);

    let row = OrderEnum::from_row(&[
        DbValue::I32(2),
        DbValue::String("Pending".into()),
        DbValue::String("Cancelled".into()),
    ])
    .unwrap();
    assert_eq!(row.status, OrderStatus::Pending);
    assert_eq!(row.archived_status, Some(OrderStatus::Cancelled));

    let snapshot = row.snapshot();
    assert_eq!(
        snapshot.get("status"),
        Some(&DbValue::String("Pending".into()))
    );
    assert_eq!(
        snapshot.get("archived_status"),
        Some(&DbValue::String("Cancelled".into()))
    );

    let meta = OrderEnum::entity_meta();
    assert_eq!(
        meta.find_property("status").unwrap().column_type.as_deref(),
        Some("VARCHAR(16)")
    );

    // `linq!` parameter conversion uses `DbValue::from(enum)`.
    let _ = linq!(|o: OrderEnum| o.status == OrderStatus::Paid);
}

#[tokio::test]
async fn sqlite_generated_column_is_computed_by_database() {
    use rust_ef_sqlite::SqliteProvider;
    use std::sync::Arc;

    let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let migration = engine
        .generate("Init", &[LineItem::entity_meta()], &None)
        .unwrap();
    engine.apply(&*provider, &migration).await.unwrap();

    let mut conn = provider.get_connection().await.unwrap();
    conn.execute(
        "INSERT INTO line_items (unit_price, quantity) VALUES (?, ?)",
        &[DbValue::I64(250), DbValue::I32(4)],
    )
    .await
    .unwrap();

    let rows = conn
        .query("SELECT total FROM line_items", &[])
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(f64::try_from(rows[0][0].clone()).unwrap(), 1000.0);
}

// Keep the `linq!` macro imported for the alternate-key API shape used above.
#[allow(dead_code)]
fn _linq_alt_key_shape() {
    let _ = linq!(index |p: Product| (p.code, p.unit_price));
}
