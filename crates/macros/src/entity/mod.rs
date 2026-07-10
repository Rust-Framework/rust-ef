//! Handles `#[derive(EntityType)]` expansion.
//!
//! The derive macro entry point lives in `expand.rs`; field parsing and
//! `EntityContext` construction in `parse.rs`; impl generation (`IEntityType`,
//! `IGetKeyValues`, `IEntitySnapshot`, `IFromRow`, `INavigationSetter`,
//! `ILazyInit`) in `gen.rs`; and attribute extraction / navigation detection
//! helpers in `helpers.rs`.

mod expand;
mod gen;
mod helpers;
mod parse;

pub use expand::expand_entity_type;
