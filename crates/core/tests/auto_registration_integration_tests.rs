//! Integration tests for `DbContext::discover_entities()` with SQLite.
//!
//! These tests verify the end-to-end flow:
//! 1. `#[derive(EntityType)]` emits `inventory::submit!`
//! 2. `#[entity(T)]` emits `inventory::submit!`
//! 3. `ctx.discover_entities()` populates STORE A and STORE B
//! 4. `ctx.ensure_created()` applies Fluent API overrides (renamed table)
//! 5. `ctx.set::<T>()` is idempotent after discovery
//! 6. Backward compatibility: `set::<T>()` without `discover_entities()` still works

use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt;

#[derive(Debug, Clone, EntityType)]
#[table("disc_int_simple")]
pub struct DiscSimple {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[required]
    #[max_length(80)]
    pub name: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("disc_int_other")]
pub struct DiscOther {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub value: i64,
}

#[derive(Default)]
pub struct DiscSimpleConfig;

#[entity(DiscSimple)]
impl IEntityTypeConfiguration<DiscSimple> for DiscSimpleConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, DiscSimple>) {
        entity.to_table("disc_int_renamed");
        entity
            .property_named("name")
            .has_column_name("display_name");
    }
}

fn build_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite(":memory:");
    let options = builder.build();
    DbContext::from_options(&options).expect("DbContext::from_options")
}

#[tokio::test]
async fn test_discover_then_ensure_created_applies_override() {
    let mut ctx = build_ctx();
    ctx.discover_entities().expect("discover_entities");

    let metas = ctx.model().build();
    let simple_meta = metas
        .iter()
        .find(|m| m.type_name.contains("DiscSimple"))
        .expect("DiscSimple meta should exist after discovery");

    assert_eq!(
        simple_meta.table_name.as_ref(),
        "disc_int_renamed",
        "to_table override should be visible in build() output"
    );

    ctx.ensure_created().await.expect("ensure_created");
}

#[tokio::test]
async fn test_set_after_discover_is_idempotent() {
    let mut ctx = build_ctx();
    ctx.discover_entities().expect("discover_entities");

    let meta_count_before = ctx.model().build().len();

    ctx.set::<DiscSimple>();
    ctx.set::<DiscOther>();

    let meta_count_after = ctx.model().build().len();

    assert_eq!(
        meta_count_before, meta_count_after,
        "set::<T>() after discover_entities() should not duplicate metas"
    );

    ctx.ensure_created().await.expect("ensure_created");
}

#[tokio::test]
async fn test_backward_compat_set_without_discover() {
    let mut ctx = build_ctx();
    ctx.set::<DiscSimple>();
    ctx.set::<DiscOther>();

    ctx.ensure_created().await.expect("ensure_created");

    ctx.set::<DiscSimple>().add(DiscSimple {
        id: 0,
        name: "alpha".into(),
    });
    ctx.set::<DiscOther>().add(DiscOther { id: 0, value: 42 });
    ctx.save_changes().await.expect("save_changes");

    let simples = ctx
        .set::<DiscSimple>()
        .query()
        .to_list()
        .await
        .expect("to_list");
    assert_eq!(simples.len(), 1);
    assert_eq!(simples[0].name, "alpha");

    let others = ctx
        .set::<DiscOther>()
        .query()
        .to_list()
        .await
        .expect("to_list");
    assert_eq!(others.len(), 1);
    assert_eq!(others[0].value, 42);
}

#[tokio::test]
async fn test_ensure_deleted_with_discover() {
    let mut ctx = build_ctx();
    ctx.discover_entities().expect("discover_entities");
    ctx.ensure_created().await.expect("ensure_created");
    ctx.ensure_deleted().await.expect("ensure_deleted");
}
