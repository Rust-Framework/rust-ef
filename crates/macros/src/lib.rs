//! Procedural macros for Rust Entity Framework (rust-ef).

mod column_macro;
mod entity;
mod linq;

use proc_macro::TokenStream;

#[proc_macro_derive(
    EntityType,
    attributes(
        table,
        primary_key,
        auto_increment,
        required,
        max_length,
        column,
        foreign_key,
        navigation,
        not_mapped,
        index,
        unique,
        through,
        concurrency_check
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
