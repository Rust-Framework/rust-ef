//! Constraint / index / default / table-option DDL coverage.
//!
//! Covers the capabilities requested in `11-mysql-code-first.md`:
//! composite unique indexes, partial (filtered) unique indexes, expression
//! indexes, composite + scalar foreign keys, CHECK constraints, column
//! defaults, `Option<String>` mapping, and MySQL table options.

use rust_ef::entity::IEntityType;
use rust_ef::migration::{MigrationDialect, MigrationEngine, MigrationStore};
use rust_ef::model_builder::{EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder};
use rust_ef::provider::{DbValue, IDatabaseProvider};
use rust_ef::relations::DeleteBehavior;
use rust_ef::{linq, EntityType};

#[derive(Debug, Clone, EntityType)]
#[table("customers")]
struct Customer {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub tenant_id: i32,
    #[max_length(64)]
    pub name: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("order_records")]
struct OrderRecord {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub tenant_id: i32,
    #[foreign_key(Customer)]
    pub customer_id: i32,
    #[max_length(64)]
    pub request_key: String,
    pub amount: i64,
    pub status: String,
    pub version: i32,
    #[max_length(64)]
    pub note: Option<String>,
}

#[derive(Default)]
struct OrderRecordConfig;

impl IEntityTypeConfiguration<OrderRecord> for OrderRecordConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, OrderRecord>) {
        // Composite unique index, built type-safely via `linq!(index ...)`.
        entity
            .has_index(linq!(index |o: OrderRecord| (o.tenant_id, o.request_key)))
            .is_unique();
        // Multi-column (non-unique) index with an explicit name.
        entity
            .has_index(&[OrderRecord::COLUMN_TENANT_ID, OrderRecord::COLUMN_STATUS])
            .has_name("ix_orders_tenant_status");
        // Expression unique index.
        entity
            .has_index_expression(&["coalesce(tenant_id, 0)"])
            .is_unique()
            .has_name("ux_orders_coalesce_tenant");
        // Composite foreign key.
        entity.has_foreign_key(
            &[OrderRecord::COLUMN_TENANT_ID, OrderRecord::COLUMN_ID],
            Customer::TABLE,
            &[Customer::COLUMN_TENANT_ID, Customer::COLUMN_ID],
        );
        // Column default + column-level CHECK.
        entity
            .property_named("amount")
            .has_default("0")
            .has_check("amount > 0");
        entity.property_named("version").has_default("1");
        // `Option<String>` with an explicit length must map to VARCHAR(n).
        entity.property_named("note").has_max_length(64);
        // Table-level CHECK.
        entity.has_check(
            "CK_orders_status",
            "status IN ('PENDING','PAID','CANCELLED')",
        );
        // MySQL table options.
        entity
            .engine("InnoDB")
            .charset("utf8mb4")
            .collate("utf8mb4_bin");
    }
}

#[derive(Debug, Clone, EntityType)]
#[table("invites")]
struct Invite {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub tenant_id: i32,
    #[max_length(32)]
    pub code: String,
    pub status: String,
}

#[derive(Default)]
struct InviteConfig;

impl IEntityTypeConfiguration<Invite> for InviteConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Invite>) {
        entity
            .has_index(&[Invite::COLUMN_TENANT_ID, Invite::COLUMN_CODE])
            .is_unique()
            .has_name("ux_invite_pending")
            .has_filter(linq!(filter |i: Invite| i.status == "PENDING"));
        entity.has_check(
            "CK_invite_status",
            "status IN ('PENDING','ACCEPTED','REVOKED')",
        );
    }
}

#[derive(Debug, Clone, EntityType)]
#[table("payments")]
struct Payment {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    // No `#[foreign_key]` attribute: the FK comes from the Fluent API only.
    pub customer_id: i32,
}

#[derive(Default)]
struct PaymentConfig;

impl IEntityTypeConfiguration<Payment> for PaymentConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Payment>) {
        entity
            .has_foreign_key(
                &[Payment::COLUMN_CUSTOMER_ID],
                Customer::TABLE,
                &[Customer::COLUMN_ID],
            )
            .on_delete(DeleteBehavior::Restrict);
    }
}

fn model() -> Vec<rust_ef::metadata::EntityTypeMeta> {
    let mut builder = ModelBuilder::new();
    builder.apply_configuration::<OrderRecordConfig, OrderRecord>();
    builder.apply_configuration::<InviteConfig, Invite>();
    builder.apply_configuration::<PaymentConfig, Payment>();
    builder.entity::<Customer>();
    builder.build()
}

#[test]
fn scalar_foreign_key_is_in_entity_metadata() {
    let meta = OrderRecord::entity_meta();
    assert_eq!(meta.foreign_keys.len(), 1, "expected one scalar FK");
    let fk = &meta.foreign_keys[0];
    assert_eq!(fk.columns, vec!["customer_id"]);
    assert_eq!(fk.referenced_table, "customers");
    assert_eq!(fk.referenced_columns, vec!["id"]);
}

#[test]
fn postgres_generates_composite_indexes_and_constraints() {
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let migration = engine.generate("Init", &model(), &None).unwrap();
    let sql = &migration.up_sql;

    // Composite unique index.
    assert!(
        sql.contains(
            "CREATE UNIQUE INDEX \"ix_order_records_tenant_id_request_key\" ON \"order_records\" (\"tenant_id\", \"request_key\")"
        ),
        "missing composite unique index:\n{sql}"
    );
    // Named composite index.
    assert!(
        sql.contains(
            "CREATE INDEX \"ix_orders_tenant_status\" ON \"order_records\" (\"tenant_id\", \"status\")"
        ),
        "missing composite index:\n{sql}"
    );
    // Expression unique index.
    assert!(
        sql.contains(
            "CREATE UNIQUE INDEX \"ux_orders_coalesce_tenant\" ON \"order_records\" ((coalesce(tenant_id, 0)))"
        ),
        "missing expression index:\n{sql}"
    );
    // Scalar FK from #[foreign_key(Customer)].
    assert!(
        sql.contains(
            "ADD CONSTRAINT \"fk_order_records_customer_id_customers\" FOREIGN KEY (\"customer_id\") REFERENCES \"customers\" (\"id\")"
        ),
        "missing scalar FK:\n{sql}"
    );
    // Composite FK.
    assert!(
        sql.contains(
            "ADD CONSTRAINT \"fk_order_records_tenant_id_id_customers\" FOREIGN KEY (\"tenant_id\", \"id\") REFERENCES \"customers\" (\"tenant_id\", \"id\")"
        ),
        "missing composite FK:\n{sql}"
    );
    // Fluent single-column FK (no `#[foreign_key]` attribute).
    assert!(
        sql.contains(
            "ADD CONSTRAINT \"fk_payments_customer_id_customers\" FOREIGN KEY (\"customer_id\") REFERENCES \"customers\" (\"id\") ON DELETE RESTRICT"
        ),
        "missing fluent single-column FK:\n{sql}"
    );
    // CHECK constraints (table-level + column-level).
    assert!(
        sql.contains(
            "CONSTRAINT \"CK_orders_status\" CHECK (status IN ('PENDING','PAID','CANCELLED'))"
        ),
        "missing table CHECK:\n{sql}"
    );
    assert!(
        sql.contains("CONSTRAINT \"CK_order_records_amount\" CHECK (amount > 0)"),
        "missing column CHECK:\n{sql}"
    );
    // Defaults.
    assert!(
        sql.contains("\"version\" INTEGER NULL DEFAULT 1"),
        "missing version default:\n{sql}"
    );
    assert!(
        sql.contains("\"amount\" BIGINT NULL DEFAULT 0"),
        "missing amount default:\n{sql}"
    );
    // Option<String> keeps VARCHAR(n).
    assert!(
        sql.contains("\"note\" VARCHAR(64)"),
        "Option<String> should map to VARCHAR(64):\n{sql}"
    );
    // Partial unique index uses WHERE on PostgreSQL.
    assert!(
        sql.contains(
            "CREATE UNIQUE INDEX \"ux_invite_pending\" ON \"invites\" (\"tenant_id\", \"code\") WHERE status = 'PENDING'"
        ),
        "missing partial index:\n{sql}"
    );
}

#[test]
fn mysql_generates_functional_partial_index_and_table_options() {
    let engine = MigrationEngine::new(MigrationDialect::MySql);
    let migration = engine.generate("Init", &model(), &None).unwrap();
    let sql = &migration.up_sql;

    // MySQL has no partial indexes: filtered UNIQUE is emulated functionally.
    assert!(
        sql.contains(
            "CREATE UNIQUE INDEX `ux_invite_pending` ON `invites` ((CASE WHEN status = 'PENDING' THEN `tenant_id` END), (CASE WHEN status = 'PENDING' THEN `code` END))"
        ),
        "missing functional partial index:\n{sql}"
    );
    // Table options are explicit.
    assert!(
        sql.contains("ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin"),
        "missing table options:\n{sql}"
    );
    // CHECK constraints are emitted inline.
    assert!(
        sql.contains(
            "CONSTRAINT `CK_orders_status` CHECK (status IN ('PENDING','PAID','CANCELLED'))"
        ),
        "missing CHECK:\n{sql}"
    );
}

#[test]
fn adding_and_removing_constraints_diffs_cleanly() {
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let old = engine.create_snapshot("old", &model());

    // Build a new model without the CHECK + composite index.
    #[derive(Default)]
    struct SlimConfig;
    impl IEntityTypeConfiguration<Invite> for SlimConfig {
        fn configure(&self, entity: &mut EntityTypeBuilder<'_, Invite>) {
            entity
                .has_index(&[Invite::COLUMN_TENANT_ID, Invite::COLUMN_CODE])
                .is_unique()
                .has_name("ux_invite_pending")
                .has_filter(linq!(filter |i: Invite| i.status == "PENDING"));
        }
    }
    let mut builder = ModelBuilder::new();
    builder.apply_configuration::<SlimConfig, Invite>();
    let migration = engine
        .generate("Change", &builder.build(), &Some(old))
        .unwrap();

    assert!(
        migration
            .up_sql
            .contains("DROP CONSTRAINT IF EXISTS \"CK_invite_status\""),
        "expected CHECK drop:\n{}",
        migration.up_sql
    );
    // Down migration restores it.
    assert!(
        migration
            .down_sql
            .contains("ADD CONSTRAINT \"CK_invite_status\" CHECK"),
        "expected CHECK restore in down:\n{}",
        migration.down_sql
    );
}

#[test]
fn generate_from_snapshot_preserves_rendered_filters() {
    let engine = MigrationEngine::new(MigrationDialect::Postgres);
    let snapshot = engine.create_snapshot("cfg", &model());
    let migration = engine
        .generate_from_snapshot("Init", &snapshot, &None)
        .unwrap();
    assert!(
        migration
            .up_sql
            .contains("CREATE UNIQUE INDEX \"ux_invite_pending\""),
        "snapshot-based generation lost the partial index:\n{}",
        migration.up_sql
    );
    assert!(
        migration.up_sql.contains("WHERE status = 'PENDING'"),
        "snapshot-based generation lost the rendered predicate:\n{}",
        migration.up_sql
    );
}

#[test]
fn snapshot_json_roundtrips_constraints() {
    let engine = MigrationEngine::new(MigrationDialect::MySql);
    let snapshot = engine.create_snapshot("constraints", &model());

    let dir = std::env::temp_dir().join("rust_ef_constraints_snapshot_test");
    let _ = std::fs::remove_dir_all(&dir);
    let store = MigrationStore::new(&dir);
    store.save_snapshot(&snapshot).unwrap();
    let loaded = store.load_snapshot().unwrap().expect("snapshot loads");

    let orders = loaded
        .entity_types
        .iter()
        .find(|et| et.table_name == "order_records")
        .expect("order_records in snapshot");
    assert_eq!(orders.indexes.len(), 3, "three declared indexes");
    assert!(orders.indexes.iter().any(|i| i.is_unique
        && i.columns.len() == 2
        && i.name == "ix_order_records_tenant_id_request_key"));
    assert!(orders
        .indexes
        .iter()
        .any(|i| i.columns.iter().any(|c| c.is_expression)));
    assert_eq!(orders.foreign_keys.len(), 1, "one composite FK");
    assert!(orders.checks.iter().any(|c| c.name == "CK_orders_status"));
    assert_eq!(
        orders.table_options.engine.as_deref(),
        Some("InnoDB"),
        "table options round-trip"
    );
    let version = orders
        .columns
        .iter()
        .find(|c| c.column_name == "version")
        .unwrap();
    assert_eq!(version.default_value.as_deref(), Some("1"));

    let invites = loaded
        .entity_types
        .iter()
        .find(|et| et.table_name == "invites")
        .expect("invites in snapshot");
    assert_eq!(
        invites.indexes[0].filter.as_deref(),
        Some("status = 'PENDING'")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn sqlite_enforces_check_and_partial_unique_index() {
    use rust_ef_sqlite::SqliteProvider;
    use std::sync::Arc;

    let provider = Arc::new(SqliteProvider::new(":memory:").unwrap());
    let engine = MigrationEngine::new(MigrationDialect::Sqlite);
    let migration = engine.generate("Init", &model(), &None).unwrap();
    engine.apply(&*provider, &migration).await.unwrap();

    let mut conn = provider.get_connection().await.unwrap();

    // Invalid status violates the CHECK constraint.
    let bad = conn
        .execute(
            "INSERT INTO invites (tenant_id, code, status) VALUES (?, ?, ?)",
            &[
                DbValue::I32(1),
                DbValue::String("A".into()),
                DbValue::String("BOGUS".into()),
            ],
        )
        .await;
    assert!(bad.is_err(), "CHECK constraint should reject BOGUS status");

    // PENDING + same (tenant, code) is rejected by the partial unique index.
    conn.execute(
        "INSERT INTO invites (tenant_id, code, status) VALUES (?, ?, ?)",
        &[
            DbValue::I32(1),
            DbValue::String("A".into()),
            DbValue::String("PENDING".into()),
        ],
    )
    .await
    .unwrap();
    let dup = conn
        .execute(
            "INSERT INTO invites (tenant_id, code, status) VALUES (?, ?, ?)",
            &[
                DbValue::I32(1),
                DbValue::String("A".into()),
                DbValue::String("PENDING".into()),
            ],
        )
        .await;
    assert!(
        dup.is_err(),
        "partial unique index should reject duplicate PENDING"
    );

    // Same code with a different status is allowed.
    conn.execute(
        "INSERT INTO invites (tenant_id, code, status) VALUES (?, ?, ?)",
        &[
            DbValue::I32(1),
            DbValue::String("A".into()),
            DbValue::String("REVOKED".into()),
        ],
    )
    .await
    .unwrap();

    // Amount default + CHECK: row with amount = 0 is rejected.
    let zero = conn
        .execute(
            "INSERT INTO order_records (tenant_id, customer_id, request_key, amount, status, version, note) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[
                DbValue::I32(1),
                DbValue::I32(1),
                DbValue::String("k1".into()),
                DbValue::I64(0),
                DbValue::String("PAID".into()),
                DbValue::I32(1),
                DbValue::Null,
            ],
        )
        .await;
    assert!(zero.is_err(), "CHECK amount > 0 should reject 0");
}
