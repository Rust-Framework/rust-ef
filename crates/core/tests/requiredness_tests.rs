//! Requiredness inference from the Rust field type (EF-12).
//!
//! A code-first model should not need `.is_required()` on every non-nullable
//! field: `Option<T>` is nullable, everything else is `NOT NULL`, and the
//! explicit Fluent override (`.is_required()` / `.is_nullable()`) still wins.

use rust_ef::entity::IEntityType;
use rust_ef::migration::{MigrationDialect, MigrationEngine};
use rust_ef::model_builder::{EntityTypeBuilder, IEntityTypeConfiguration, ModelBuilder};
use rust_ef::EntityType;

#[derive(Debug, Clone, EntityType)]
#[table("req_users")]
struct ReqUser {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    /// Non-`Option` without `#[required]` → inferred `NOT NULL`.
    pub email: String,
    /// `Option<T>` → nullable.
    pub nickname: Option<String>,
    /// Explicit `#[required]` on `Option<T>` → `NOT NULL`.
    #[required]
    pub tenant_id: Option<i32>,
    /// Non-`Option`, overridden to nullable through the Fluent API below.
    pub score: i64,
}

#[derive(Default)]
struct ReqUserConfig;

impl IEntityTypeConfiguration<ReqUser> for ReqUserConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, ReqUser>) {
        // Explicit override beats the type inference.
        entity.property_named("score").is_nullable();
    }
}

fn generated_ddl(dialect: MigrationDialect, metas: &[rust_ef::metadata::EntityTypeMeta]) -> String {
    MigrationEngine::new(dialect)
        .generate("InitialCreate", metas, &None)
        .expect("generate")
        .up_sql
}

#[test]
fn derive_meta_infers_requiredness_from_the_field_type() {
    let meta = ReqUser::entity_meta();
    let property = |name: &str| {
        meta.properties
            .iter()
            .find(|p| p.field_name == name)
            .unwrap_or_else(|| panic!("missing property {name}"))
    };

    assert!(property("email").is_required, "String must be required");
    assert!(
        !property("nickname").is_required,
        "Option<String> must be optional"
    );
    assert!(
        property("tenant_id").is_required,
        "#[required] must override Option<T>"
    );
    assert!(property("score").is_required, "i64 must be required");
}

#[test]
fn non_option_columns_are_not_null_in_every_dialect() {
    let metas = [ReqUser::entity_meta()];
    for dialect in [
        MigrationDialect::Sqlite,
        MigrationDialect::Postgres,
        MigrationDialect::MySql,
    ] {
        let sql = generated_ddl(dialect, &metas);
        let q = if dialect == MigrationDialect::MySql {
            "`"
        } else {
            "\""
        };
        assert!(
            sql.contains(&format!("{q}email{q} TEXT NOT NULL")),
            "{dialect:?} should mark a non-Option String NOT NULL:\n{sql}"
        );
        assert!(
            sql.contains(&format!("{q}nickname{q} TEXT NULL")),
            "{dialect:?} should keep Option<String> nullable:\n{sql}"
        );
        assert!(
            sql.contains(&format!("{q}tenant_id{q} INTEGER NOT NULL")),
            "{dialect:?} should honour #[required] on Option<T>:\n{sql}"
        );
        assert!(
            sql.contains(&format!("{q}score{q} BIGINT NOT NULL")),
            "{dialect:?} should mark a non-Option i64 NOT NULL:\n{sql}"
        );
    }
}

#[test]
fn explicit_nullable_override_wins_over_type_inference() {
    let mut builder = ModelBuilder::new();
    builder.apply_configuration::<ReqUserConfig, ReqUser>();
    let metas = builder.build();

    let sql = generated_ddl(MigrationDialect::Postgres, &metas);
    assert!(
        sql.contains("\"score\" BIGINT NULL"),
        "is_nullable() must override the i64 inference:\n{sql}"
    );
    // The override only affects the configured column.
    assert!(
        sql.contains("\"email\" TEXT NOT NULL"),
        "other columns keep their inferred requiredness:\n{sql}"
    );
}
