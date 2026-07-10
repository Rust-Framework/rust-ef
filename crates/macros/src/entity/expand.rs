//! Derive macro entry point — validates input and orchestrates parse + gen.

use proc_macro::TokenStream;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

use super::gen::generate_impls;
use super::helpers::{extract_context_key, extract_table_name};
use super::parse::parse_fields;

pub fn expand_entity_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    let table_name = extract_table_name(&input.attrs);
    let context_key_tokens = extract_context_key(&input.attrs)
        .unwrap_or_else(|| quote::quote! { ::core::option::Option::None });

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return syn::Error::new_spanned(
                    &input.ident,
                    "EntityType can only be derived for structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(
                &input.ident,
                "EntityType can only be derived for structs",
            )
            .to_compile_error()
            .into();
        }
    };

    let ctx = match parse_fields(struct_name, &table_name, fields) {
        Ok(ctx) => ctx,
        Err(e) => return e.to_compile_error().into(),
    };

    let expanded = generate_impls(&ctx, fields, context_key_tokens);
    TokenStream::from(expanded)
}
