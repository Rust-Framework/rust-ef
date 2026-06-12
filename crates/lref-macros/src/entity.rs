//! Handles `#[derive(EntityType)]` expansion.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, GenericArgument, LitStr, PathArguments, Type};

pub fn expand_entity_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let struct_name_str = struct_name.to_string();

    let table_name = extract_table_name(&input.attrs);

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

    let mut property_builders = Vec::new();
    let mut navigation_builders = Vec::new();
    let mut primary_key_names = Vec::new();
    let mut from_row_fields = Vec::new();
    let mut nav_field_names = Vec::new();
    let mut pk_field_idents: Vec<&syn::Ident> = Vec::new(); // primary key field names

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let field_type = &field.ty;

        let is_navigation = is_navigation_field(field_type);
        let is_not_mapped = has_attr(&field.attrs, "not_mapped");
        let is_primary_key = has_attr(&field.attrs, "primary_key");
        let is_auto_increment = has_attr(&field.attrs, "auto_increment");
        let is_required = has_attr(&field.attrs, "required");
        let is_foreign_key = has_attr(&field.attrs, "foreign_key");
        let is_concurrency_token = has_attr(&field.attrs, "concurrency_check");
        let is_unique = has_attr(&field.attrs, "unique");
        let has_index = has_attr(&field.attrs, "index");
        let max_length = extract_max_length(&field.attrs);
        let column_name = extract_column_name(&field.attrs, &field_name_str);

        if is_primary_key {
            primary_key_names.push(quote! { std::borrow::Cow::Borrowed(#field_name_str) });
            pk_field_idents.push(field_name);
        }

        if is_navigation {
            let (nav_kind, inner_type) = detect_navigation_kind_and_inner(field_type);
            let nav_kind_token = match nav_kind {
                NavigationDiscriminant::BelongsTo => {
                    quote! { lref::metadata::NavigationKind::BelongsTo }
                }
                NavigationDiscriminant::HasOne => {
                    quote! { lref::metadata::NavigationKind::HasOne }
                }
                NavigationDiscriminant::HasMany => {
                    quote! { lref::metadata::NavigationKind::HasMany }
                }
            };
            let fk_field = extract_foreign_key_field_name(&field.attrs);

            navigation_builders.push(quote! {
                lref::metadata::NavigationMeta {
                    field_name: std::borrow::Cow::Borrowed(#field_name_str),
                    kind: #nav_kind_token,
                    related_type_id: std::any::TypeId::of::<#inner_type>(),
                    related_type_name: std::borrow::Cow::Borrowed(std::any::type_name::<#inner_type>()),
                    foreign_key_field: #fk_field,
                    inverse_navigation: None,
                    through_type_id: None,
                }
            });
            nav_field_names.push(field_name);
        } else if !is_not_mapped {
            property_builders.push(quote! {
                lref::metadata::PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed(#field_name_str),
                    column_name: std::borrow::Cow::Borrowed(#column_name),
                    type_id: std::any::TypeId::of::<#field_type>(),
                    type_name: std::borrow::Cow::Borrowed(std::any::type_name::<#field_type>()),
                    is_primary_key: #is_primary_key,
                    is_auto_increment: #is_auto_increment,
                    is_required: #is_required,
                    is_foreign_key: #is_foreign_key,
                    is_concurrency_token: #is_concurrency_token,
                    max_length: #max_length,
                    is_unique: #is_unique,
                    has_index: #has_index,
                    is_not_mapped: false,
                }
            });
            // Collect for FromRow generation
            from_row_fields.push((field_name, field_type));
        }
    }

    // Generate FromRow field parsers for scalar fields
    let mut from_row_assignments = Vec::new();
    let mut column_consts = Vec::new();

    for (idx, (field_name, field_type)) in from_row_fields.iter().enumerate() {
        let idx_lit = syn::Index::from(idx);
        let type_str = quote!(#field_type).to_string();

        let parse_expr = generate_parse_expr(field_type, &type_str, idx_lit);
        from_row_assignments.push(quote! {
            #field_name: #parse_expr,
        });
    }

    // Generate column name constants for all mapped scalar fields
    // Collect the column names from property_builders (before they're consumed)
    // We need to re-derive column names here
    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let is_nav = is_navigation_field(field_ty);
        let is_nm = has_attr(&field.attrs, "not_mapped");

        if !is_nav && !is_nm {
            let col_name = extract_column_name(&field.attrs, &field_name.to_string());
            let const_name = syn::Ident::new(
                &format!("COLUMN_{}", field_name.to_string().to_uppercase()),
                field_name.span(),
            );
            column_consts.push(quote! {
                pub const #const_name: &'static str = #col_name;
            });
        }
    }

    // Add default values for navigation fields
    for field_name in &nav_field_names {
        from_row_assignments.push(quote! {
            #field_name: Default::default(),
        });
    }

    let type_name_str = struct_name_str;
    let field_count = from_row_fields.len();

    // Build snapshot assignments: field_name -> DbValue::from(self.field)
    let mut snapshot_entries = Vec::new();
    for (field_name, _field_type) in &from_row_fields {
        let field_name_str = field_name.to_string();
        snapshot_entries.push(quote! {
            map.insert(
                #field_name_str.to_string(),
                lref::provider::DbValue::from(self.#field_name.clone()),
            );
        });
    }

    let expanded = quote! {
        impl lref::entity::IEntityType for #struct_name {
            fn entity_meta() -> lref::metadata::EntityTypeMeta {
                lref::metadata::EntityTypeMeta {
                    type_id: std::any::TypeId::of::<Self>(),
                    type_name: std::borrow::Cow::Borrowed(#type_name_str),
                    table_name: std::borrow::Cow::Borrowed(#table_name),
                    properties: vec![
                        #(#property_builders,)*
                    ],
                    navigations: vec![
                        #(#navigation_builders,)*
                    ],
                    primary_keys: vec![
                        #(#primary_key_names,)*
                    ],
                }
            }
        }

        impl #struct_name {
            #(#column_consts)*
        }

        impl lref::entity::IGetKeyValues for #struct_name {
            fn key_values(&self) -> std::collections::HashMap<String, lref::provider::DbValue> {
                let mut map = std::collections::HashMap::new();
                #(
                    map.insert(
                        stringify!(#pk_field_idents).to_string(),
                        lref::provider::DbValue::from(self.#pk_field_idents.clone()),
                    );
                )*
                map
            }
        }

        impl lref::entity::IEntitySnapshot for #struct_name {
            fn snapshot(&self) -> std::collections::HashMap<String, lref::provider::DbValue> {
                let mut map = std::collections::HashMap::new();
                #(#snapshot_entries)*
                map
            }
        }

        impl lref::entity::IFromRow for #struct_name {
            fn from_row(values: &[String]) -> lref::error::LrefResult<Self> {
                if values.len() < #field_count {
                    return Err(lref::error::LrefError::TypeConversion(
                        format!("Expected {} columns, got {}", #field_count, values.len())
                    ));
                }
                Ok(#struct_name {
                    #(#from_row_assignments)*
                })
            }
        }
    };

    TokenStream::from(expanded)
}

// ---------------------------------------------------------------------------
// Navigation detection helpers
// ---------------------------------------------------------------------------

enum NavigationDiscriminant {
    BelongsTo,
    HasOne,
    HasMany,
}

/// Detects the navigation kind (BelongsTo/HasOne/HasMany) from a type
/// like `BelongsTo<Blog>`, `HasMany<Post>`, `HasOne<User>`.
///
/// Extracts both the discriminant and the inner entity type from the
/// angle-bracketed generic argument.
fn detect_navigation_kind_and_inner(ty: &Type) -> (NavigationDiscriminant, syn::Type) {
    let unit_type: syn::Type = syn::parse_quote! { () };

    if let Type::Path(type_path) = ty {
        if let Some(segment) = type_path.path.segments.last() {
            let ident = segment.ident.to_string();

            let kind = if ident.starts_with("BelongsTo") {
                NavigationDiscriminant::BelongsTo
            } else if ident.starts_with("HasMany") {
                NavigationDiscriminant::HasMany
            } else if ident.starts_with("HasOne") {
                NavigationDiscriminant::HasOne
            } else {
                NavigationDiscriminant::BelongsTo
            };

            let inner = if let PathArguments::AngleBracketed(args) = &segment.arguments {
                args.args
                    .first()
                    .and_then(|a| {
                        if let GenericArgument::Type(inner_ty) = a {
                            Some(inner_ty.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| unit_type.clone())
            } else {
                unit_type.clone()
            };

            return (kind, inner);
        }
    }

    (NavigationDiscriminant::BelongsTo, unit_type)
}

/// Extracts the target type name from `#[foreign_key(TargetType)]`
/// and produces `Some(Cow::Borrowed("TargetType"))`.
fn extract_foreign_key_field_name(attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
    for attr in attrs {
        if attr.path().is_ident("foreign_key") {
            if let syn::Meta::List(list) = &attr.meta {
                let target = list.tokens.to_string().trim().to_string();
                return quote! { Some(std::borrow::Cow::Borrowed(#target)) };
            }
        }
    }
    quote! { None }
}

// ---------------------------------------------------------------------------
// Attribute extraction helpers
// ---------------------------------------------------------------------------

fn extract_table_name(attrs: &[syn::Attribute]) -> String {
    for attr in attrs {
        if attr.path().is_ident("table") {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                return lit_str.value();
            }
        }
    }
    String::new()
}

fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn extract_column_name(attrs: &[syn::Attribute], field_name: &str) -> String {
    for attr in attrs {
        if attr.path().is_ident("column") {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                return lit_str.value();
            }
        }
    }
    field_name.to_string()
}

fn extract_max_length(attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
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

fn is_navigation_field(ty: &Type) -> bool {
    let type_str = quote!(#ty).to_string();
    type_str.contains("BelongsTo") || type_str.contains("HasMany") || type_str.contains("HasOne")
}

/// Generates the parse expression for converting a row value string into a Rust type.
/// Supports: i32, i64, i16, f64, f32, bool, String, Option<T>
fn generate_parse_expr(ty: &Type, type_str: &str, idx: syn::Index) -> proc_macro2::TokenStream {
    // Check if it's Option<T>
    if type_str.starts_with("Option <") || type_str.starts_with("Option<") {
        // Extract inner type from Option<T>
        let inner_str = if let Type::Path(type_path) = ty {
            if let Some(seg) = type_path.path.segments.last() {
                if let PathArguments::AngleBracketed(args) = &seg.arguments {
                    if let Some(GenericArgument::Type(inner_ty)) = args.args.first() {
                        let inner_type_str = quote!(#inner_ty).to_string();
                        let inner_parse = generate_scalar_parse(&inner_type_str, inner_ty, idx.clone());
                        return quote! {
                            {
                                let v = &values[#idx];
                                if v.is_empty() || v == "NULL" {
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
            values[#idx].parse::<i32>().unwrap_or(0)
        },
        "i64" | "i 64" => quote! {
            values[#idx].parse::<i64>().unwrap_or(0)
        },
        "i16" | "i 16" => quote! {
            values[#idx].parse::<i16>().unwrap_or(0)
        },
        "i8" | "i 8" => quote! {
            values[#idx].parse::<i8>().unwrap_or(0)
        },
        "u32" | "u 32" => quote! {
            values[#idx].parse::<u32>().unwrap_or(0)
        },
        "u64" | "u 64" => quote! {
            values[#idx].parse::<u64>().unwrap_or(0)
        },
        "f64" | "f 64" => quote! {
            values[#idx].parse::<f64>().unwrap_or(0.0)
        },
        "f32" | "f 32" => quote! {
            values[#idx].parse::<f32>().unwrap_or(0.0)
        },
        "bool" => quote! {
            values[#idx].parse::<bool>().unwrap_or(false)
        },
        "String" => quote! {
            values[#idx].clone()
        },
        "Vec < u8 >" | "Vec<u8>" => quote! {
            values[#idx].as_bytes().to_vec()
        },
        _ => {
            // Default to String for unknown types
            quote! {
                values[#idx].clone()
            }
        }
    }
}
