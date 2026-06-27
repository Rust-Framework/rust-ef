//! Tests for inventory-based automatic entity registration.
//!
//! Verifies that `#[derive(EntityType)]` emits `inventory::submit!` for
//! `EntityRegistration`, that `#[entity(T)]` emits
//! `EntityConfigRegistration`, and that `DbContext::discover_entities()`
//! populates both STORE A and STORE B from the global registry.

#![cfg(test)]

use rust_ef::prelude::*;
use rust_ef::registration::{EntityConfigRegistration, EntityRegistration};

#[derive(Debug, Clone, EntityType)]
#[table("auto_reg_simple")]
pub struct SimpleEntity {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[required]
    #[max_length(100)]
    pub name: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("auto_reg_other")]
pub struct OtherEntity {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub value: i64,
}

#[derive(Default)]
pub struct SimpleEntityConfig;

#[entity(SimpleEntity)]
impl IEntityTypeConfiguration<SimpleEntity> for SimpleEntityConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, SimpleEntity>) {
        entity.to_table("auto_reg_renamed");
        entity
            .property_named("name")
            .has_column_name("display_name");
    }
}

#[test]
fn test_entity_registration_exists() {
    let registrations: Vec<&EntityRegistration> = inventory::iter::<EntityRegistration>().collect();

    let has_simple = registrations
        .iter()
        .any(|r| r.type_name.contains("SimpleEntity"));
    let has_other = registrations
        .iter()
        .any(|r| r.type_name.contains("OtherEntity"));

    assert!(
        has_simple,
        "SimpleEntity should be registered via inventory"
    );
    assert!(has_other, "OtherEntity should be registered via inventory");
}

#[test]
fn test_entity_config_registration_exists() {
    let configs: Vec<&EntityConfigRegistration> =
        inventory::iter::<EntityConfigRegistration>().collect();

    let has_simple_config = configs.iter().any(|r| r.type_name.contains("SimpleEntity"));

    assert!(
        has_simple_config,
        "SimpleEntityConfig should be registered via inventory"
    );
}

#[test]
fn test_inventory_iter_non_empty() {
    let count = inventory::iter::<EntityRegistration>().count();
    assert!(count >= 2, "Should have at least 2 registered entities");
}

#[test]
fn test_model_builder_build_applies_config() {
    let mut builder = ModelBuilder::new();

    for reg in inventory::iter::<EntityConfigRegistration> {
        (reg.apply_fn)(&mut builder);
    }

    for reg in inventory::iter::<EntityRegistration> {
        if !builder.has_entity(reg.type_id) {
            builder.register_entity_meta(reg.meta());
        }
    }

    let metas = builder.build();

    let simple_meta = metas
        .iter()
        .find(|m| m.type_name.contains("SimpleEntity"))
        .expect("SimpleEntity meta should exist in build() output");

    assert_eq!(
        simple_meta.table_name.as_ref(),
        "auto_reg_renamed",
        "to_table override from #[entity] should be applied"
    );

    let name_prop = simple_meta
        .properties
        .iter()
        .find(|p| p.field_name.as_ref() == "name")
        .expect("name property should exist");

    assert_eq!(
        name_prop.column_name.as_ref(),
        "display_name",
        "has_column_name override should be applied"
    );
}

#[test]
fn test_other_entity_keeps_default_table_name() {
    let mut builder = ModelBuilder::new();

    for reg in inventory::iter::<EntityRegistration> {
        if !builder.has_entity(reg.type_id) {
            builder.register_entity_meta(reg.meta());
        }
    }

    let metas = builder.build();

    let other_meta = metas
        .iter()
        .find(|m| m.type_name.contains("OtherEntity"))
        .expect("OtherEntity meta should exist");

    assert_eq!(
        other_meta.table_name.as_ref(),
        "auto_reg_other",
        "Entities without #[entity] should keep their #[table] name"
    );
}

#[test]
fn test_register_entity_meta_is_idempotent() {
    let mut builder = ModelBuilder::new();
    let meta = SimpleEntity::entity_meta();
    let type_id = meta.type_id;

    builder.register_entity_meta(meta.clone());
    builder.register_entity_meta(meta.clone());

    let count = builder
        .build()
        .iter()
        .filter(|m| m.type_id == type_id)
        .count();

    assert_eq!(count, 1, "register_entity_meta should be idempotent");
}

#[test]
fn test_has_entity_returns_correct_value() {
    let mut builder = ModelBuilder::new();
    let meta = SimpleEntity::entity_meta();
    let type_id = meta.type_id;

    assert!(!builder.has_entity(type_id));
    builder.register_entity_meta(meta);
    assert!(builder.has_entity(type_id));
}
