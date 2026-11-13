//! Procedural macros for Rust Entity Framework (rust-ef).

#![forbid(unsafe_code)]

mod column_macro;
mod crate_path;
mod db_context_key;
mod db_enum;
mod entity;
mod entity_config;
mod linq;

use proc_macro::TokenStream;

#[proc_macro_derive(
    EntityType,
    attributes(
        table,
        primary_key,
        auto_increment,
        sequence,
        required,
        max_length,
        column,
        foreign_key,
        navigation,
        not_mapped,
        index,
        unique,
        through,
        concurrency_check,
        on_delete,
        context,
        keyless,
        column_type,
        precision,
        fixed_length,
        collation,
        comment,
        computed
    )
)]
pub fn derive_entity_type(input: TokenStream) -> TokenStream {
    entity::expand_entity_type(input)
}

#[proc_macro]
pub fn column(input: TokenStream) -> TokenStream {
    column_macro::expand_column(input)
}

/// Derive a typed keyed-`DbContext` key.
///
/// The enum's variants become `IDbContextKey::name()` values (snake-case by
/// default, overridable with `#[key("...")]`) and the type implements
/// `From<Key> for rust_dix::ServiceKey`. This makes
/// `#[inject(owned, key = Key::Read)]` and `add_dbcontext_keyed(Key::Read, ...)`
/// share the same typed key.
///
/// ```ignore
/// #[derive(DbContextKey, Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// enum Db {
///     Read,
///     Write,
///     #[key("audit-log")]
///     Audit,
/// }
/// ```
///
/// The enum must derive `Debug + Clone + Copy + PartialEq + Eq + Hash` (the
/// bounds required by `rust_dix::ServiceKey::new`).
#[proc_macro_derive(DbContextKey, attributes(key))]
pub fn derive_db_context_key(input: TokenStream) -> TokenStream {
    db_context_key::expand(syn::parse_macro_input!(input as syn::DeriveInput)).into()
}

/// Derive string-backed database mapping for a unit-only enum.
///
/// Generates `From<Enum> for DbValue`, `TryFrom<DbValue> for Enum`,
/// `as_str()` and `from_db_str()`. Variant names default to the Rust variant
/// identifier; override with `#[db_value("...")]`.
///
/// ```ignore
/// #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, DbEnum)]
/// enum Status {
///     #[default]
///     Pending,
///     #[db_value("PAID")]
///     Paid,
/// }
/// ```
#[proc_macro_derive(DbEnum, attributes(db_value))]
pub fn derive_db_enum(input: TokenStream) -> TokenStream {
    db_enum::expand_db_enum(input)
}

/// Compile-time LINQ-to-SQL.
///
/// ```ignore
/// linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;
///
/// let expr = linq!(|b: Blog| b.rating > min);
/// ctx.set::<Blog>().filter(expr).to_list().await?;
/// ```
#[proc_macro]
pub fn linq(input: TokenStream) -> TokenStream {
    linq::expand_linq(input)
}

/// Attribute macro for `impl IEntityTypeConfiguration<T>` blocks.
///
/// Emits an `inventory::submit!` registering the configuration for automatic
/// discovery by `DbContext::from_options()`. The first argument is the entity
/// type `T`; the config type is taken from the `impl`'s `Self` type. An
/// optional second argument specifies the DbContext key for multi-database
/// scenarios.
///
/// # Examples
///
/// ```ignore
/// #[derive(Default)]
/// pub struct BlogConfig;
///
/// // Default context
/// #[entity(Blog)]
/// impl IEntityTypeConfiguration<Blog> for BlogConfig {
///     fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
///         entity.to_table("blogs_renamed");
///     }
/// }
///
/// // Keyed context ("logs" database)
/// #[entity(LogEntry, "logs")]
/// impl LogEntryConfig for IEntityTypeConfiguration<LogEntry> {
///     fn configure(&self, entity: &mut EntityTypeBuilder<'_, LogEntry>) {
///         entity.to_table("app_logs");
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn entity(args: TokenStream, input: TokenStream) -> TokenStream {
    entity_config::expand_entity_config(args, input)
}
