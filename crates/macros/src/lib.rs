//! Procedural macros for Rust Entity Framework (rust-ef).

mod column_macro;
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
        context
    )
)]
pub fn derive_entity_type(input: TokenStream) -> TokenStream {
    entity::expand_entity_type(input)
}

#[proc_macro]
pub fn column(input: TokenStream) -> TokenStream {
    column_macro::expand_column(input)
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
