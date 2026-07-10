//! Attribute extraction and navigation type detection helpers.

use quote::quote;
use syn::{GenericArgument, LitStr, PathArguments, Type};

// ---------------------------------------------------------------------------
// Navigation detection helpers
// ---------------------------------------------------------------------------

pub(crate) enum NavigationDiscriminant {
    BelongsTo,
    HasOne,
    HasMany,
    ManyToMany,
}

pub(crate) struct NavTypeInfo {
    pub(crate) kind: NavigationDiscriminant,
    pub(crate) related: syn::Type,
    pub(crate) join: Option<syn::Type>,
}

pub(crate) fn type_ident_string(ty: &syn::Type) -> String {
    if let syn::Type::Path(p) = ty {
        p.path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "Entity".to_string())
    } else {
        "Entity".to_string()
    }
}

fn is_unit_type(ty: &syn::Type) -> bool {
    quote!(#ty).to_string().replace(' ', "") == "()"
}

/// Detects navigation kind and related/join types from `BelongsTo<T>`, `HasMany<T, J>`, etc.
pub(crate) fn detect_navigation_type(ty: &syn::Type) -> NavTypeInfo {
    let unit_type: syn::Type = syn::parse_quote! { () };

    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident = segment.ident.to_string();

            let kind = if ident.starts_with("BelongsTo") {
                NavigationDiscriminant::BelongsTo
            } else if ident.starts_with("HasOne") {
                NavigationDiscriminant::HasOne
            } else if ident.starts_with("HasMany") {
                NavigationDiscriminant::HasMany
            } else {
                NavigationDiscriminant::BelongsTo
            };

            let mut related_ty = unit_type.clone();
            let mut join_ty = None;

            if let PathArguments::AngleBracketed(args) = &segment.arguments {
                if let Some(GenericArgument::Type(first)) = args.args.first() {
                    related_ty = first.clone();
                }
                if ident.starts_with("HasMany") {
                    if let Some(GenericArgument::Type(second)) = args.args.get(1) {
                        if !is_unit_type(second) {
                            join_ty = Some(second.clone());
                        }
                    }
                }
            }

            let final_kind = if ident.starts_with("HasMany") && join_ty.is_some() {
                NavigationDiscriminant::ManyToMany
            } else {
                kind
            };

            return NavTypeInfo {
                kind: final_kind,
                related: related_ty,
                join: join_ty,
            };
        }
    }

    NavTypeInfo {
        kind: NavigationDiscriminant::BelongsTo,
        related: unit_type,
        join: None,
    }
}

/// Extracts the target type name from `#[foreign_key(TargetType)]`
/// and produces `Some(Cow::Borrowed("TargetType"))`.
pub(crate) fn extract_foreign_key_target(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("foreign_key") {
            if let syn::Meta::List(list) = &attr.meta {
                return Some(list.tokens.to_string().trim().to_string());
            }
        }
    }
    None
}

/// Parses `#[through(JoinEntity)]` on a navigation property.
pub(crate) fn extract_through_type(attrs: &[syn::Attribute]) -> Option<syn::Type> {
    for attr in attrs {
        if attr.path().is_ident("through") {
            if let syn::Meta::List(list) = &attr.meta {
                let tokens = list.tokens.to_string();
                return syn::parse_str::<syn::Type>(&tokens).ok();
            }
        }
    }
    None
}

/// Resolves the `foreign_key_field` metadata for a *navigation* property.
///
/// Historically this read `#[foreign_key(X)]` on navigation fields and stored `X`
/// (the target type name, e.g. `"Post"`) into `NavigationMeta.foreign_key_field` —
/// a bug, since that field's documented semantics is the FK *property name* on the
/// dependent entity (e.g. `"post_id"`), not a type name.
///
/// The `#[foreign_key(Target)]` attribute belongs on *scalar* FK columns (where it
/// generates the `FK_<Target>` constant via `extract_foreign_key_target`). Navigation
/// properties derive their FK column from the relationship kind at runtime
/// (`navigation_loader` uses `fk_column` / `referenced_key_column`, not this field).
///
/// We therefore no longer consult `#[foreign_key]` on navigation fields and return
/// `None`. A dedicated `#[fk_field(name)]` attribute can be introduced later should
/// explicit override become necessary. `_attrs` is retained for signature stability.
pub(crate) fn extract_foreign_key_field_name(
    _attrs: &[syn::Attribute],
) -> proc_macro2::TokenStream {
    quote! { None }
}

// ---------------------------------------------------------------------------
// Attribute extraction helpers
// ---------------------------------------------------------------------------

pub(crate) fn extract_table_name(attrs: &[syn::Attribute]) -> String {
    for attr in attrs {
        if attr.path().is_ident("table") {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                return lit_str.value();
            }
        }
    }
    String::new()
}

/// Extracts the optional `#[context("key")]` attribute for multi-database
/// scenarios. Returns `None` for the default context, `Some("key")` for a
/// keyed context.
pub(crate) fn extract_context_key(attrs: &[syn::Attribute]) -> Option<proc_macro2::TokenStream> {
    for attr in attrs {
        if attr.path().is_ident("context") {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                let key = lit_str.value();
                return Some(quote! { ::core::option::Option::Some(#key) });
            }
        }
    }
    Some(quote! { ::core::option::Option::None })
}

pub(crate) fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

pub(crate) fn extract_column_name(attrs: &[syn::Attribute], field_name: &str) -> String {
    for attr in attrs {
        if attr.path().is_ident("column") {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                return lit_str.value();
            }
        }
    }
    field_name.to_string()
}

pub(crate) fn extract_sequence_name(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("sequence") {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                return Some(lit_str.value());
            }
        }
    }
    None
}

pub(crate) fn extract_on_delete(attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
    for attr in attrs {
        if attr.path().is_ident("on_delete") {
            let tokens = &attr.meta;
            if let syn::Meta::List(list) = tokens {
                let behavior: String = list.tokens.to_string().trim().to_string();
                return match behavior.as_str() {
                    "Cascade" => quote! { Some(rust_ef::relations::DeleteBehavior::Cascade) },
                    "Restrict" => quote! { Some(rust_ef::relations::DeleteBehavior::Restrict) },
                    "SetNull" => quote! { Some(rust_ef::relations::DeleteBehavior::SetNull) },
                    "NoAction" => quote! { Some(rust_ef::relations::DeleteBehavior::NoAction) },
                    _ => quote! { None },
                };
            }
        }
    }
    quote! { None }
}

pub(crate) fn extract_max_length(attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
    for attr in attrs {
        if attr.path().is_ident("max_length") {
            if let Ok(lit_int) = attr.parse_args::<syn::LitInt>() {
                let n: usize = lit_int.base10_parse().unwrap_or(0);
                return quote! { Some(#n) };
            }
        }
    }
    quote! { None }
}

pub(crate) fn is_navigation_field(ty: &Type) -> bool {
    let type_str = quote!(#ty).to_string();
    type_str.contains("BelongsTo") || type_str.contains("HasMany") || type_str.contains("HasOne")
}

/// Generates the parse expression for converting a row value string into a Rust type.
/// Supports: i32, i64, i16, f64, f32, bool, String, Option<T>
pub(crate) fn generate_parse_expr(
    ty: &Type,
    type_str: &str,
    idx: syn::Index,
) -> proc_macro2::TokenStream {
    // Check if it's Option<T>
    if type_str.starts_with("Option <") || type_str.starts_with("Option<") {
        // Extract inner type from Option<T>
        let inner_str = if let Type::Path(type_path) = ty {
            if let Some(seg) = type_path.path.segments.last() {
                if let PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        let inner_type_str = quote!(#inner_ty).to_string();
                        let inner_parse =
                            generate_scalar_parse(&inner_type_str, inner_ty, idx.clone());
                        return quote! {
                            {
                                let v = &values[#idx];
                                if matches!(v, rust_ef::provider::DbValue::Null) {
                                    None
                                } else {
                                    Some(#inner_parse)
                                }
                            }
                        };
                    }
                }
            }
            quote! { None }
        } else {
            quote! { None }
        };
        return inner_str;
    }

    generate_scalar_parse(type_str, ty, idx)
}

fn generate_scalar_parse(type_str: &str, _ty: &Type, idx: syn::Index) -> proc_macro2::TokenStream {
    match type_str {
        "i32" | "i 32" => quote! {
            values[#idx].clone().try_into().unwrap_or(0i32)
        },
        "i64" | "i 64" => quote! {
            values[#idx].clone().try_into().unwrap_or(0i64)
        },
        "i16" | "i 16" => quote! {
            values[#idx].clone().try_into().unwrap_or(0i16)
        },
        "i8" | "i 8" => quote! {
            values[#idx].clone().try_into().unwrap_or(0i8)
        },
        "u32" | "u 32" => quote! {
            values[#idx].clone().try_into().unwrap_or(0u32)
        },
        "u64" | "u 64" => quote! {
            values[#idx].clone().try_into().unwrap_or(0u64)
        },
        "f64" | "f 64" => quote! {
            values[#idx].clone().try_into().unwrap_or(0.0f64)
        },
        "f32" | "f 32" => quote! {
            values[#idx].clone().try_into().unwrap_or(0.0f32)
        },
        "bool" => quote! {
            values[#idx].clone().try_into().unwrap_or(false)
        },
        "String" => quote! {
            values[#idx].clone().try_into().unwrap_or_default()
        },
        "Vec < u8 >" | "Vec<u8>" => quote! {
            values[#idx].clone().try_into().unwrap_or_default()
        },
        // chrono types — order matters: NaiveDateTime before DateTime, NaiveDate before Date
        _ if cfg!(feature = "chrono") && type_str.contains("NaiveDateTime") => quote! {
            values[#idx].clone().try_into().unwrap_or_default()
        },
        _ if cfg!(feature = "chrono") && type_str.contains("NaiveDate") => quote! {
            values[#idx].clone().try_into().unwrap_or_default()
        },
        _ if cfg!(feature = "chrono") && type_str.contains("DateTime") => quote! {
            values[#idx].clone().try_into().unwrap_or_default()
        },
        // uuid::Uuid
        _ if cfg!(feature = "uuid") && type_str.contains("Uuid") => quote! {
            values[#idx].clone().try_into().unwrap_or_default()
        },
        // rust_decimal::Decimal
        _ if cfg!(feature = "decimal") && type_str.contains("Decimal") => quote! {
            values[#idx].clone().try_into().unwrap_or_default()
        },
        _ => {
            quote! {
                values[#idx].clone().try_into().unwrap_or_default()
            }
        }
    }
}
