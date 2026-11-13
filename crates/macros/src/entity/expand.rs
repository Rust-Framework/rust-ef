//! Derive macro entry point — validates input and orchestrates parse + gen.

use proc_macro::TokenStream;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

use super::gen::generate_impls;
use super::helpers::{extract_context_key, extract_table_args, has_attr};
use super::parse::parse_fields;

pub fn expand_entity_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let ef = crate::crate_path::ef_crate_path();

    let table_args = match extract_table_args(&input.attrs) {
        Ok(args) => args,
        Err(err) => return err.to_compile_error().into(),
    };
    let (table_name, is_view, table_comment) = match table_args {
        Some(args) => {
            let mut name = args.name;
            // `schema = "audit"` qualifies the table (and navigation
            // `related_table`, because `TABLE` carries the qualified name).
            if let Some(schema) = args.schema {
                if !name.is_empty() && !name.contains('.') {
                    name = format!("{schema}.{name}");
                }
            }
            (name, args.view, args.comment)
        }
        None => (String::new(), false, None),
    };
    let is_keyless = has_attr(&input.attrs, "keyless");
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

    let ctx = match parse_fields(struct_name, &table_name, fields, &ef) {
        Ok(ctx) => ctx,
        Err(e) => return e.to_compile_error().into(),
    };

    let expanded = generate_impls(
        &ctx,
        fields,
        context_key_tokens,
        is_view,
        is_keyless,
        table_comment,
        &ef,
    );
    TokenStream::from(expanded)
}
