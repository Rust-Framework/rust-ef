//! MACRO-1 regression: expansions must address the framework crate through its
//! *real* dependency name instead of hardcoding `rust_ef::…`.
//!
//! `rust-ef-macros`'s manifest declares the core crate as
//! `ef = { package = "rust-ef", path = "../core" }`. If any expansion still
//! emits `rust_ef::…`, this file fails to compile — exactly the failure Travel
//! hit before adding `extern crate ef as rust_ef;` shims.
//!
//! Deliberately no `extern crate ef as rust_ef;` here.

use ef::prelude::*;

// ─────────────────────────────────────────────────────────────────────────
// `#[derive(EntityType)]` — IEntityType / snapshots / navigation impls
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, EntityType)]
#[table("rename_widgets")]
pub struct Widget {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[required]
    pub name: String,
    #[navigation]
    pub parts: HasMany<WidgetPart>,
}

#[derive(Debug, Clone, EntityType)]
#[table("rename_widget_parts")]
pub struct WidgetPart {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[foreign_key(Widget)]
    pub widget_id: i32,
    #[navigation]
    pub widget: BelongsTo<Widget>,
}

#[test]
fn entity_derive_resolves_renamed_crate() {
    assert_eq!(Widget::TABLE, "rename_widgets");
    let meta = <Widget as ef::entity::IEntityType>::entity_meta();
    assert_eq!(meta.table_name.as_ref(), "rename_widgets");
    // Navigation metadata + IFromRow snapshots are generated in the same
    // expansion; touching them proves those paths compiled.
    assert_eq!(
        <Widget as ef::entity::IEntityType>::entity_meta()
            .navigations
            .len(),
        1
    );
}

// ─────────────────────────────────────────────────────────────────────────
// `#[derive(DbEnum)]` — DbValue conversions
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Default, DbEnum)]
pub enum WidgetState {
    #[default]
    #[db_value("ACTIVE")]
    Active,
    Disabled,
}

#[test]
fn db_enum_derive_resolves_renamed_crate() {
    let value: ef::provider::DbValue = WidgetState::Active.into();
    assert_eq!(value, ef::provider::DbValue::String("ACTIVE".to_string()));
    assert_eq!(
        WidgetState::from_db_str("Disabled"),
        Some(WidgetState::Disabled)
    );
    assert_eq!(WidgetState::Active.as_str(), "ACTIVE");
}

// ─────────────────────────────────────────────────────────────────────────
// `#[derive(DbContextKey)]` — IDbContextKey + ServiceKey conversion
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, DbContextKey)]
pub enum WidgetDb {
    #[key("read-only")]
    Read,
    Write,
}

#[test]
fn db_context_key_derive_resolves_renamed_crate() {
    use ef::di::IDbContextKey;
    assert_eq!(WidgetDb::Read.name(), "read-only");
    assert_eq!(WidgetDb::Write.name(), "write");
    let _key: ef::di::ServiceKey = WidgetDb::Write.into();
}

// ─────────────────────────────────────────────────────────────────────────
// `#[entity(T)]` attribute — IEntityTypeConfiguration inventory registration
// ─────────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct WidgetConfig;

#[ef::entity(Widget)]
impl ef::model_builder::IEntityTypeConfiguration<Widget> for WidgetConfig {
    fn configure(&self, entity: &mut ef::model_builder::EntityTypeBuilder<'_, Widget>) {
        entity.to_table("rename_widgets_configured");
    }
}

#[test]
fn entity_attribute_resolves_renamed_crate() {
    let mut found = false;
    for reg in ef::inventory::iter::<ef::registration::EntityConfigRegistration> {
        if reg.type_name == "Widget" {
            found = true;
        }
    }
    assert!(found, "entity config registration not submitted");
}

// ─────────────────────────────────────────────────────────────────────────
// `linq!` — BoolExpr / Predicate codegen
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn linq_expands_to_renamed_crate_paths() {
    let predicate: ef::query::Predicate<Widget> =
        ef::linq!(|w: Widget| w.id > 0 && w.name.contains("a"));
    drop(predicate);
}
