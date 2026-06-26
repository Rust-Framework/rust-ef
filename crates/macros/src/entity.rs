//! Handles `#[derive(EntityType)]` expansion.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, Data, DeriveInput, Fields, GenericArgument, LitStr, PathArguments, Type,
};

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
    let mut pk_column_names: Vec<String> = Vec::new();
    let mut fk_column_names: Vec<String> = Vec::new();
    let mut from_row_fields = Vec::new();
    let mut nav_field_names = Vec::new();
    let mut pk_field_idents: Vec<&syn::Ident> = Vec::new();
    let mut has_many_setter_arms = Vec::new();
    let mut reference_setter_arms = Vec::new();
    let mut nested_loader_arms = Vec::new();
    let mut fk_const_decls = Vec::new();
    let mut fk_index_arms = Vec::new();
    let mut fk_target_arms = Vec::new();
    let mut pk_column_name_lit = quote! { "id" };
    let mut pk_column_index_lit = quote! { 0usize };

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
            pk_column_names.push(column_name.clone());
            pk_column_name_lit = quote! { #column_name };
        }

        if is_navigation {
            let mut nav_info = detect_navigation_type(field_type);
            if let Some(through_ty) = extract_through_type(&field.attrs) {
                nav_info.join = Some(through_ty);
                nav_info.kind = NavigationDiscriminant::ManyToMany;
            }
            let inner_type = &nav_info.related;
            let nav_kind = nav_info.kind;
            let nav_kind_token = match nav_kind {
                NavigationDiscriminant::BelongsTo => {
                    quote! { rust_ef::metadata::NavigationKind::BelongsTo }
                }
                NavigationDiscriminant::HasOne => {
                    quote! { rust_ef::metadata::NavigationKind::HasOne }
                }
                NavigationDiscriminant::HasMany => {
                    quote! { rust_ef::metadata::NavigationKind::HasMany }
                }
                NavigationDiscriminant::ManyToMany => {
                    quote! { rust_ef::metadata::NavigationKind::ManyToMany }
                }
            };
            let fk_field = extract_foreign_key_field_name(&field.attrs);
            let parent_pk_col = pk_column_names.first().map(|s| s.as_str()).unwrap_or("id");
            let parent_fk_col = fk_column_names.first().map(|s| s.as_str()).unwrap_or("id");
            let parent_type_name = struct_name.to_string();
            let related_type_name = type_ident_string(inner_type);
            let fk_const = syn::Ident::new(&format!("FK_{}", parent_type_name), struct_name.span());

            navigation_builders.push(match nav_kind {
                NavigationDiscriminant::ManyToMany => {
                    let join_type = nav_info.join.as_ref().expect("ManyToMany requires join type");
                    let _ = related_type_name;
                    quote! {
                        rust_ef::metadata::NavigationMeta {
                            field_name: std::borrow::Cow::Borrowed(#field_name_str),
                            kind: #nav_kind_token,
                            related_type_id: std::any::TypeId::of::<#inner_type>(),
                            related_type_name: std::borrow::Cow::Borrowed(std::any::type_name::<#inner_type>()),
                            foreign_key_field: #fk_field,
                            inverse_navigation: None,
                            through_type_id: Some(std::any::TypeId::of::<#join_type>()),
                            through_table: Some(std::borrow::Cow::Borrowed(<#join_type>::TABLE)),
                            through_parent_fk: {
                                <#join_type>::fk_column_for(std::any::TypeId::of::<#struct_name>())
                                    .map(std::borrow::Cow::Borrowed)
                            },
                            through_related_fk: {
                                <#join_type>::fk_column_for(std::any::TypeId::of::<#inner_type>())
                                    .map(std::borrow::Cow::Borrowed)
                            },
                            through_parent_fk_index: <#join_type>::fk_column_for(std::any::TypeId::of::<#struct_name>())
                                .map(|c| <#join_type>::fk_column_index(c))
                                .unwrap_or(0),
                            through_related_fk_index: <#join_type>::fk_column_for(std::any::TypeId::of::<#inner_type>())
                                .map(|c| <#join_type>::fk_column_index(c))
                                .unwrap_or(0),
                            related_table: Some(std::borrow::Cow::Borrowed(<#inner_type>::TABLE)),
                            fk_column: None,
                            referenced_key_column: Some(std::borrow::Cow::Borrowed(<#inner_type>::pk_column_name())),
                            fk_row_index: 0,
                            pk_row_index: <#inner_type>::pk_column_index(),
                            related_entity_meta: Some(<#inner_type as rust_ef::entity::IEntityType>::entity_meta),
                        }
                    }
                }
                NavigationDiscriminant::HasMany => quote! {
                    rust_ef::metadata::NavigationMeta {
                        field_name: std::borrow::Cow::Borrowed(#field_name_str),
                        kind: #nav_kind_token,
                        related_type_id: std::any::TypeId::of::<#inner_type>(),
                        related_type_name: std::borrow::Cow::Borrowed(std::any::type_name::<#inner_type>()),
                        foreign_key_field: #fk_field,
                        inverse_navigation: None,
                        through_type_id: None,
                        through_table: None,
                        through_parent_fk: None,
                        through_related_fk: None,
                        through_parent_fk_index: 0,
                        through_related_fk_index: 0,
                        related_table: Some(std::borrow::Cow::Borrowed(<#inner_type>::TABLE)),
                        fk_column: Some(std::borrow::Cow::Borrowed(<#inner_type>::#fk_const)),
                        referenced_key_column: Some(std::borrow::Cow::Borrowed(#parent_pk_col)),
                        fk_row_index: <#inner_type>::fk_column_index(stringify!(#fk_const)),
                        pk_row_index: <#inner_type>::pk_column_index(),
                        related_entity_meta: Some(<#inner_type as rust_ef::entity::IEntityType>::entity_meta),
                    }
                },
                NavigationDiscriminant::BelongsTo | NavigationDiscriminant::HasOne => quote! {
                    rust_ef::metadata::NavigationMeta {
                        field_name: std::borrow::Cow::Borrowed(#field_name_str),
                        kind: #nav_kind_token,
                        related_type_id: std::any::TypeId::of::<#inner_type>(),
                        related_type_name: std::borrow::Cow::Borrowed(std::any::type_name::<#inner_type>()),
                        foreign_key_field: #fk_field,
                        inverse_navigation: None,
                        through_type_id: None,
                        through_table: None,
                        through_parent_fk: None,
                        through_related_fk: None,
                        through_parent_fk_index: 0,
                        through_related_fk_index: 0,
                        related_table: Some(std::borrow::Cow::Borrowed(<#inner_type>::TABLE)),
                        fk_column: Some(std::borrow::Cow::Borrowed(#parent_fk_col)),
                        referenced_key_column: Some(std::borrow::Cow::Borrowed(
                            <#inner_type>::pk_column_name(),
                        )),
                        fk_row_index: Self::fk_column_index(#parent_fk_col),
                        pk_row_index: <#inner_type>::pk_column_index(),
                        related_entity_meta: Some(<#inner_type as rust_ef::entity::IEntityType>::entity_meta),
                    }
                },
            });

            match nav_kind {
                NavigationDiscriminant::HasMany | NavigationDiscriminant::ManyToMany => {
                    has_many_setter_arms.push(quote! {
                        if field == #field_name_str {
                            let items: rust_ef::error::EFResult<Vec<#inner_type>> = rows
                                .iter()
                                .map(|r| <#inner_type as rust_ef::entity::IFromRow>::from_row(r))
                                .collect();
                            self.#field_name = rust_ef::relations::HasMany::with(items?);
                            return Ok(());
                        }
                    });
                    nested_loader_arms.push(quote! {
                        if parent_navigation == #field_name_str && !nested.is_empty() {
                            let children = self.#field_name.items_mut();
                            rust_ef::navigation_loader::load_includes(children, nested, provider, filter_map).await?;
                            for path in nested {
                                if !path.nested.is_empty() {
                                    for child in children.iter_mut() {
                                        child.load_nested_includes(&path.navigation, &path.nested, provider, filter_map).await?;
                                    }
                                }
                            }
                            return Ok(());
                        }
                    });
                }
                NavigationDiscriminant::BelongsTo => {
                    reference_setter_arms.push(quote! {
                        if field == #field_name_str {
                            self.#field_name = rust_ef::relations::BelongsTo::with(
                                <#inner_type as rust_ef::entity::IFromRow>::from_row(row)?,
                            );
                            return Ok(());
                        }
                    });
                    nested_loader_arms.push(quote! {
                        if parent_navigation == #field_name_str && !nested.is_empty() {
                            if let Some(related) = self.#field_name.get_mut() {
                                rust_ef::navigation_loader::load_includes(
                                    std::slice::from_mut(related),
                                    nested,
                                    provider,
                                    filter_map,
                                ).await?;
                                for path in nested {
                                    if !path.nested.is_empty() {
                                        related.load_nested_includes(&path.navigation, &path.nested, provider, filter_map).await?;
                                    }
                                }
                            }
                            return Ok(());
                        }
                    });
                }
                NavigationDiscriminant::HasOne => {
                    reference_setter_arms.push(quote! {
                        if field == #field_name_str {
                            self.#field_name = rust_ef::relations::HasOne::with(
                                <#inner_type as rust_ef::entity::IFromRow>::from_row(row)?,
                            );
                            return Ok(());
                        }
                    });
                    nested_loader_arms.push(quote! {
                        if parent_navigation == #field_name_str && !nested.is_empty() {
                            if let Some(related) = self.#field_name.get_mut() {
                                rust_ef::navigation_loader::load_includes(
                                    std::slice::from_mut(related),
                                    nested,
                                    provider,
                                    filter_map,
                                ).await?;
                                for path in nested {
                                    if !path.nested.is_empty() {
                                        related.load_nested_includes(&path.navigation, &path.nested, provider, filter_map).await?;
                                    }
                                }
                            }
                            return Ok(());
                        }
                    });
                }
            }
            nav_field_names.push(field_name);
        } else if !is_not_mapped {
            let scalar_idx = from_row_fields.len();
            if is_primary_key {
                pk_column_index_lit = quote! { #scalar_idx };
            }
            property_builders.push(quote! {
                rust_ef::metadata::PropertyMeta {
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
            if is_foreign_key {
                fk_column_names.push(column_name.clone());
                if let Some(target) = extract_foreign_key_target(&field.attrs) {
                    let target_ident = syn::Ident::new(&target, field_name.span());
                    let fk_ident = syn::Ident::new(&format!("FK_{}", target), field_name.span());
                    let col = column_name.clone();
                    fk_const_decls.push(quote! {
                        #[allow(non_upper_case_globals)]
                        pub const #fk_ident: &'static str = #col;
                    });
                    fk_index_arms.push(quote! {
                        #col | stringify!(#fk_ident) => #scalar_idx,
                    });
                    fk_target_arms.push(quote! {
                        if target == std::any::TypeId::of::<#target_ident>() {
                            return Some(#col);
                        }
                    });
                }
            }
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

    // Generate navigation field name constants (`FIELD_*`) so that `linq!` can
    // reference navigation members type-safely, e.g. `include b.posts` expands to
    // `Blog::FIELD_POSTS` which is `&'static str = "posts"` passed to
    // `find_navigation(...)`. Mirrors the `COLUMN_*` convention above.
    let mut nav_field_consts: Vec<proc_macro2::TokenStream> = Vec::new();
    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        if is_navigation_field(field_ty) {
            let name_str = field_name.to_string();
            let const_name = syn::Ident::new(
                &format!("FIELD_{}", name_str.to_uppercase()),
                field_name.span(),
            );
            nav_field_consts.push(quote! {
                pub const #const_name: &'static str = #name_str;
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
                rust_ef::provider::DbValue::from(self.#field_name.clone()),
            );
        });
    }

    let expanded = quote! {
        impl rust_ef::entity::IEntityType for #struct_name {
            fn entity_meta() -> rust_ef::metadata::EntityTypeMeta {
                rust_ef::metadata::EntityTypeMeta {
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
            pub const TABLE: &'static str = #table_name;
            #(#column_consts)*
            #(#nav_field_consts)*
            #(#fk_const_decls)*

            pub fn pk_column_name() -> &'static str {
                #pk_column_name_lit
            }

            pub fn pk_column_index() -> usize {
                #pk_column_index_lit
            }

            pub fn fk_column_index(col: &str) -> usize {
                match col {
                    #( #fk_index_arms )*
                    _ => 0,
                }
            }

            /// Resolves the FK column pointing at `target` (many-to-many join lookup).
            pub fn fk_column_for(target: std::any::TypeId) -> Option<&'static str> {
                #( #fk_target_arms )*
                None
            }
        }

        impl rust_ef::entity::IGetKeyValues for #struct_name {
            fn key_values(&self) -> std::collections::HashMap<String, rust_ef::provider::DbValue> {
                let mut map = std::collections::HashMap::new();
                #(
                    map.insert(
                        stringify!(#pk_field_idents).to_string(),
                        rust_ef::provider::DbValue::from(self.#pk_field_idents.clone()),
                    );
                )*
                map
            }
        }

        impl rust_ef::entity::IEntitySnapshot for #struct_name {
            fn snapshot(&self) -> std::collections::HashMap<String, rust_ef::provider::DbValue> {
                let mut map = std::collections::HashMap::new();
                #(#snapshot_entries)*
                map
            }
        }

        impl rust_ef::entity::IFromRow for #struct_name {
            fn from_row(values: &[String]) -> rust_ef::error::EFResult<Self> {
                if values.len() < #field_count {
                    return Err(rust_ef::error::EFError::TypeConversion(
                        format!("Expected {} columns, got {}", #field_count, values.len())
                    ));
                }
                Ok(#struct_name {
                    #(#from_row_assignments)*
                })
            }
        }

        #[rust_ef::async_trait::async_trait]
        impl rust_ef::entity::INavigationSetter for #struct_name {
            fn apply_has_many(
                &mut self,
                field: &str,
                rows: &[Vec<String>],
            ) -> rust_ef::error::EFResult<()> {
                #( #has_many_setter_arms )*
                Ok(())
            }

            fn apply_reference(
                &mut self,
                field: &str,
                row: &[String],
            ) -> rust_ef::error::EFResult<()> {
                #( #reference_setter_arms )*
                Ok(())
            }

            async fn load_nested_includes(
                &mut self,
                parent_navigation: &str,
                nested: &[rust_ef::query::IncludePath],
                provider: &dyn rust_ef::provider::IDatabaseProvider,
                filter_map: ::core::option::Option<&std::collections::HashMap<String, rust_ef::query::BoolExpr>>,
            ) -> rust_ef::error::EFResult<()> {
                #( #nested_loader_arms )*
                Ok(())
            }
        }

        rust_ef::inventory::submit!({
            rust_ef::registration::EntityRegistration {
                type_id: std::any::TypeId::of::<#struct_name>(),
                type_name: stringify!(#struct_name),
                meta_fn: <#struct_name as rust_ef::entity::IEntityType>::entity_meta,
            }
        });
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
    ManyToMany,
}

struct NavTypeInfo {
    kind: NavigationDiscriminant,
    related: syn::Type,
    join: Option<syn::Type>,
}

fn type_ident_string(ty: &syn::Type) -> String {
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
fn detect_navigation_type(ty: &syn::Type) -> NavTypeInfo {
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
fn extract_foreign_key_target(attrs: &[syn::Attribute]) -> Option<String> {
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
fn extract_through_type(attrs: &[syn::Attribute]) -> Option<syn::Type> {
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
fn extract_foreign_key_field_name(_attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
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
                        let inner_parse =
                            generate_scalar_parse(&inner_type_str, inner_ty, idx.clone());
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
