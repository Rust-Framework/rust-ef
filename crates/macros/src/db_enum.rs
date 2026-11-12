//! `#[derive(DbEnum)]` — map a unit-only Rust enum to a string database value.
//!
//! Generates the two trait impls the rest of rust-ef needs for a custom property
//! type (mirroring EF Core's `HasConversion<string>()`):
//!
//! - `From<Enum> for DbValue` (INSERT/UPDATE/`linq!` parameters)
//! - `TryFrom<DbValue> for Enum` (row materialization)
//!
//! and two convenience methods:
//!
//! - `Enum::as_str(&self) -> &'static str`
//! - `Enum::from_db_str(&str) -> Option<Self>`
//!
//! Variant names default to the Rust variant identifier; override with
//! `#[db_value("PENDING")]`.
//!
//! The `EntityType` derive's row parser falls back to `Default` for unknown
//! scalar types, so the enum must also derive `Default` (usually
//! `#[derive(Default)]` with `#[default]` on one variant).

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, LitStr};

pub fn expand_db_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_ident = input.ident.clone();

    let Data::Enum(data) = &input.data else {
        return syn::Error::new_spanned(&input.ident, "DbEnum can only be derived for enums")
            .to_compile_error()
            .into();
    };

    let mut from_arms = Vec::new();
    let mut try_arms = Vec::new();
    let mut as_str_arms = Vec::new();
    let mut from_str_arms = Vec::new();

    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return syn::Error::new_spanned(
                variant,
                "DbEnum only supports unit variants (no tuple/struct fields)",
            )
            .to_compile_error()
            .into();
        }
        let variant_ident = &variant.ident;
        let value = variant
            .attrs
            .iter()
            .find(|a| a.path().is_ident("db_value"))
            .and_then(|a| a.parse_args::<LitStr>().ok())
            .map(|s| s.value())
            .unwrap_or_else(|| variant_ident.to_string());

        let path = quote! { #enum_ident::#variant_ident };
        from_arms.push(quote! {
            #path => rust_ef::provider::DbValue::String(#value.to_string()),
        });
        try_arms.push(quote! {
            #value => ::core::result::Result::Ok(#path),
        });
        as_str_arms.push(quote! {
            #path => #value,
        });
        from_str_arms.push(quote! {
            #value => ::core::option::Option::Some(#path),
        });
    }

    let enum_name = enum_ident.to_string();
    let expanded = quote! {
        impl ::core::convert::From<#enum_ident> for rust_ef::provider::DbValue {
            fn from(value: #enum_ident) -> Self {
                match value {
                    #(#from_arms)*
                }
            }
        }

        impl ::core::convert::TryFrom<rust_ef::provider::DbValue> for #enum_ident {
            type Error = rust_ef::provider::DbValueConvertError;

            fn try_from(
                value: rust_ef::provider::DbValue,
            ) -> ::core::result::Result<Self, Self::Error> {
                match value {
                    rust_ef::provider::DbValue::String(s) => match s.as_str() {
                        #(#try_arms)*
                        _ => ::core::result::Result::Err(
                            rust_ef::provider::DbValueConvertError {
                                source: rust_ef::provider::DbValue::String(s),
                                target_type: #enum_name,
                            },
                        ),
                    },
                    other => ::core::result::Result::Err(
                        rust_ef::provider::DbValueConvertError {
                            source: other,
                            target_type: #enum_name,
                        },
                    ),
                }
            }
        }

        impl #enum_ident {
            /// Returns the database string value for this variant.
            pub fn as_str(&self) -> &'static str {
                match self {
                    #(#as_str_arms)*
                }
            }

            /// Parses a database string value; `None` for unknown values.
            pub fn from_db_str(value: &str) -> ::core::option::Option<Self> {
                match value {
                    #(#from_str_arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    };

    TokenStream::from(expanded)
}
