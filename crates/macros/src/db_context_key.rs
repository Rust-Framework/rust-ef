//! `#[derive(DbContextKey)]` — typed keyed-context keys.
//!
//! Generates:
//! - `IDbContextKey` (string identity used by `#[context("...")]` and
//!   `#[entity(T, "key")]`),
//! - `From<Key> for rust_dix::ServiceKey` (the official rust-dix extension
//!   contract), so `#[inject(owned, key = Key::Read)]` and keyed registration
//!   share the same typed service key.
//!
//! Keys default to the snake-case variant name; override with
//! `#[key("custom")]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, LitStr, Variant};

pub(crate) fn expand(input: DeriveInput) -> TokenStream {
    match expand_inner(input) {
        Ok(tokens) => tokens,
        Err(err) => err.to_compile_error(),
    }
}

fn expand_inner(input: DeriveInput) -> syn::Result<TokenStream> {
    let ident = input.ident;
    let data = match input.data {
        Data::Enum(data) => data,
        _ => {
            return Err(Error::new_spanned(
                ident,
                "DbContextKey can only be derived for enums",
            ))
        }
    };

    let mut arms = Vec::new();
    for variant in data.variants {
        match &variant.fields {
            Fields::Unit => {}
            _ => {
                return Err(Error::new_spanned(
                    &variant,
                    "DbContextKey variants must be unit variants",
                ))
            }
        }
        let name = key_name(&variant)?;
        let variant_ident = variant.ident;
        arms.push(quote! { Self::#variant_ident => #name });
    }

    Ok(quote! {
        impl ::rust_ef::di::IDbContextKey for #ident {
            fn name(&self) -> &str {
                match self {
                    #(#arms),*
                }
            }
        }

        impl ::core::convert::From<#ident> for ::rust_ef::di::ServiceKey {
            fn from(value: #ident) -> Self {
                ::rust_ef::di::ServiceKey::new(value)
            }
        }
    })
}

fn key_name(variant: &Variant) -> syn::Result<LitStr> {
    for attr in &variant.attrs {
        if attr.path().is_ident("key") {
            return attr.parse_args::<LitStr>();
        }
    }
    Ok(LitStr::new(
        &to_snake_case(&variant.ident.to_string()),
        variant.ident.span(),
    ))
}

fn to_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if index != 0 {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(ch);
        }
    }
    out
}
