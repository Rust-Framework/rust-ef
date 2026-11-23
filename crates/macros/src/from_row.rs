//! `#[derive(FromRow)]` — positional row mapping into a named-field struct.
//!
//! Generates `impl rust_ef::row::IRow for Struct`, mapping the struct fields in
//! declaration order onto the positional column values (`values[0]`,
//! `values[1]`, ...). Each field is converted with `TryFrom<DbValue>`.
//!
//! Use this derive for report/raw-SQL rows that are **not** entities. Entities
//! get their `IRow` implementation from `#[derive(EntityType)]` (they also
//! support name-based materialization via `IRow::from_row_named` when column
//! order is not deterministic); deriving both on one struct would produce two
//! `IRow` implementations and does not compile.
//!
//! Mapping is positional because providers return `Vec<DbValue>` rows without
//! column names: the `SELECT` list order must match the field declaration
//! order.
//!
//! A row whose length differs from the field count (too short **or** too long) is
//! rejected with a typed `EFError::TypeConversion`
//! naming the missing field and its column index; a bad cell reports the field
//! name, index and the conversion error.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

pub fn expand_from_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let ef = crate::crate_path::ef_crate_path();
    let struct_ident = input.ident.clone();

    let Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&struct_ident, "FromRow can only be derived for structs")
            .to_compile_error()
            .into();
    };
    let Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(
            &struct_ident,
            "FromRow requires a struct with named fields",
        )
        .to_compile_error()
        .into();
    };
    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &input.generics,
            "FromRow does not support generic structs",
        )
        .to_compile_error()
        .into();
    }

    let struct_name = struct_ident.to_string();
    let field_names: Vec<syn::Ident> = fields
        .named
        .iter()
        .map(|field| {
            field
                .ident
                .clone()
                .expect("named fields always have identifiers")
        })
        .collect();
    let field_types: Vec<&syn::Type> = fields.named.iter().map(|field| &field.ty).collect();
    let field_indexes: Vec<usize> = (0..fields.named.len()).collect();
    let field_count = fields.named.len();

    let expanded = quote! {
        impl #ef::row::IRow for #struct_ident {
            fn from_row(values: &[#ef::provider::DbValue]) -> #ef::error::EFResult<Self> {
                const FIELD_COUNT: usize = #field_count;
                const FIELD_NAMES: [&str; FIELD_COUNT] = [#(stringify!(#field_names)),*];
                if values.len() > FIELD_COUNT {
                    // A longer row means the SELECT list and the struct have
                    // drifted; silently ignoring the extra columns would hide
                    // that, so the mapping is strict in both directions.
                    return ::core::result::Result::Err(
                        #ef::error::EFError::type_conversion(::std::format!(
                            "row has {} value(s) but `{}` expects exactly {}; column {} has no field",
                            values.len(),
                            #struct_name,
                            FIELD_COUNT,
                            FIELD_COUNT
                        ))
                    );
                }
                if values.len() < FIELD_COUNT {
                    // Guarded by the length check above, so the index is in
                    // bounds; report the first field that has no value.
                    let missing_index = values.len();
                    let missing_name = FIELD_NAMES[missing_index];
                    return ::core::result::Result::Err(
                        #ef::error::EFError::type_conversion(::std::format!(
                            "row has {} value(s) but `{}` needs {} (missing field `{}` at column {})",
                            values.len(),
                            #struct_name,
                            FIELD_COUNT,
                            missing_name,
                            missing_index
                        ))
                    );
                }
                ::core::result::Result::Ok(Self {
                    #(
                        #field_names: <#field_types as ::core::convert::TryFrom<
                            #ef::provider::DbValue
                        >>::try_from(values[#field_indexes].clone()).map_err(|error| {
                            #ef::error::EFError::type_conversion(::std::format!(
                                "`{}` column {} for field `{}` cannot be converted: {}",
                                #struct_name,
                                #field_indexes,
                                stringify!(#field_names),
                                error
                            ))
                        })?,
                    )*
                })
            }
        }
    };

    TokenStream::from(expanded)
}
