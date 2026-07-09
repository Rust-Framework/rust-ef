//! Handles `#[derive(EntityType)]` expansion.

use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{
    parse_macro_input, Data, DeriveInput, Fields, GenericArgument, LitStr, PathArguments, Type,
};

pub fn expand_entity_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;
    let struct_name_str = struct_name.to_string();

    let table_name = extract_table_name(&input.attrs);
    let context_key_tokens = extract_context_key(&input.attrs);

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
    let mut lazy_init_arms: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut pk_field_idents: Vec<&syn::Ident> = Vec::new();
    let mut has_many_setter_arms = Vec::new();
    let mut reference_setter_arms = Vec::new();
    let mut nested_loader_arms = Vec::new();
    let mut drain_has_many_arms = Vec::new();
    let mut fk_const_decls = Vec::new();
    let mut fk_index_arms = Vec::new();
    let mut fk_target_arms = Vec::new();
    let mut set_fk_arms = Vec::new();
    let mut pk_column_name_lit = quote! { "id" };
    let mut pk_column_index_lit = quote! { 0usize };
    let mut auto_inc_pk_ident: Option<&syn::Ident> = None;

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_name_str = field_name.to_string();
        let field_type = &field.ty;

        let is_navigation = is_navigation_field(field_type);
        let is_not_mapped = has_attr(&field.attrs, "not_mapped");
        let is_primary_key = has_attr(&field.attrs, "primary_key");
        let is_auto_increment = has_attr(&field.attrs, "auto_increment");
        let sequence_name = extract_sequence_name(&field.attrs);
        let is_sequence = sequence_name.is_some();
        if is_auto_increment && is_sequence {
            return syn::Error::new(
                field.span(),
                "#[auto_increment] and #[sequence] are mutually exclusive",
            )
            .to_compile_error()
            .into();
        }
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
            if is_auto_increment || is_sequence {
                auto_inc_pk_ident = Some(field_name);
            }
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
            let delete_behavior_tokens = extract_on_delete(&field.attrs);

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
                            delete_behavior: #delete_behavior_tokens,
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
                        delete_behavior: #delete_behavior_tokens,
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
                        delete_behavior: #delete_behavior_tokens,
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
                    drain_has_many_arms.push(quote! {
                        if field == #field_name_str {
                            let items = std::mem::take(self.#field_name.items_mut());
                            if items.is_empty() {
                                return ::core::option::Option::None;
                            }
                            return ::core::option::Option::Some(
                                items.into_iter()
                                    .map(|item| Box::new(item)
                                        as Box<dyn std::any::Any + Send + Sync>)
                                    .collect(),
                            );
                        }
                    });
                    nested_loader_arms.push(quote! {
                        if parent_navigation == #field_name_str && !nested.is_empty() {
                            let mut counts: Vec<usize> = Vec::with_capacity(entities.len());
                            let mut all_children: Vec<#inner_type> = Vec::new();
                            for entity in entities.iter_mut() {
                                let items = std::mem::take(entity.#field_name.items_mut());
                                counts.push(items.len());
                                all_children.extend(items);
                            }
                            if !all_children.is_empty() {
                                rust_ef::navigation_loader::load_includes(
                                    &mut all_children, nested, provider, filter_map,
                                ).await?;
                                for path in nested {
                                    if !path.nested.is_empty() {
                                        <#inner_type as rust_ef::entity::INavigationSetter>::load_nested_includes(
                                            &mut all_children,
                                            &path.navigation,
                                            &path.nested,
                                            provider,
                                            filter_map,
                                        ).await?;
                                    }
                                }
                            }
                            for (entity, &count) in entities.iter_mut().zip(&counts) {
                                let items: Vec<#inner_type> = all_children.drain(..count).collect();
                                *entity.#field_name.items_mut() = items;
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
                            let mut slots: Vec<Option<#inner_type>> = entities.iter_mut()
                                .map(|e| e.#field_name.take())
                                .collect();
                            let mut all_related: Vec<#inner_type> = Vec::new();
                            let mut positions: Vec<Option<usize>> = Vec::with_capacity(slots.len());
                            for slot in slots.iter_mut() {
                                if let Some(entity) = slot.take() {
                                    positions.push(Some(all_related.len()));
                                    all_related.push(entity);
                                } else {
                                    positions.push(None);
                                }
                            }
                            if !all_related.is_empty() {
                                rust_ef::navigation_loader::load_includes(
                                    &mut all_related, nested, provider, filter_map,
                                ).await?;
                                for path in nested {
                                    if !path.nested.is_empty() {
                                        <#inner_type as rust_ef::entity::INavigationSetter>::load_nested_includes(
                                            &mut all_related,
                                            &path.navigation,
                                            &path.nested,
                                            provider,
                                            filter_map,
                                        ).await?;
                                    }
                                }
                            }
                            let mut iter = all_related.into_iter();
                            for (entity, pos) in entities.iter_mut().zip(positions) {
                                if pos.is_some() {
                                    entity.#field_name = rust_ef::relations::BelongsTo::with(iter.next().unwrap());
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
                            let mut slots: Vec<Option<#inner_type>> = entities.iter_mut()
                                .map(|e| e.#field_name.take())
                                .collect();
                            let mut all_related: Vec<#inner_type> = Vec::new();
                            let mut positions: Vec<Option<usize>> = Vec::with_capacity(slots.len());
                            for slot in slots.iter_mut() {
                                if let Some(entity) = slot.take() {
                                    positions.push(Some(all_related.len()));
                                    all_related.push(entity);
                                } else {
                                    positions.push(None);
                                }
                            }
                            if !all_related.is_empty() {
                                rust_ef::navigation_loader::load_includes(
                                    &mut all_related, nested, provider, filter_map,
                                ).await?;
                                for path in nested {
                                    if !path.nested.is_empty() {
                                        <#inner_type as rust_ef::entity::INavigationSetter>::load_nested_includes(
                                            &mut all_related,
                                            &path.navigation,
                                            &path.nested,
                                            provider,
                                            filter_map,
                                        ).await?;
                                    }
                                }
                            }
                            let mut iter = all_related.into_iter();
                            for (entity, pos) in entities.iter_mut().zip(positions) {
                                if pos.is_some() {
                                    entity.#field_name = rust_ef::relations::HasOne::with(iter.next().unwrap());
                                }
                            }
                            return Ok(());
                        }
                    });
                }
            }
            nav_field_names.push(field_name);
            // Generate the lazy-init arm: looks up this navigation's meta
            // and attaches a LazyContextImpl to the container.
            lazy_init_arms.push(quote! {
                if let Some(nav) = meta.find_navigation(#field_name_str) {
                    let ctx = rust_ef::lazy::LazyContextImpl::new(
                        std::sync::Arc::clone(&provider),
                        snapshot.clone(),
                        key_values.clone(),
                        nav.clone(),
                        filter_map.clone(),
                        depth,
                    );
                    self.#field_name.set_lazy_context(std::sync::Arc::new(ctx));
                }
            });
        } else if !is_not_mapped {
            let scalar_idx = from_row_fields.len();
            if is_primary_key {
                pk_column_index_lit = quote! { #scalar_idx };
            }
            let sequence_name_lit = match &sequence_name {
                Some(name) => quote! { Some(std::borrow::Cow::Borrowed(#name)) },
                None => quote! { None },
            };
            property_builders.push(quote! {
                rust_ef::metadata::PropertyMeta {
                    field_name: std::borrow::Cow::Borrowed(#field_name_str),
                    column_name: std::borrow::Cow::Borrowed(#column_name),
                    type_id: std::any::TypeId::of::<#field_type>(),
                    type_name: std::borrow::Cow::Borrowed(std::any::type_name::<#field_type>()),
                    is_primary_key: #is_primary_key,
                    is_auto_increment: #is_auto_increment,
                    is_sequence: #is_sequence,
                    sequence_name: #sequence_name_lit,
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
                    set_fk_arms.push({
                        let field_type_str = quote!(#field_type).to_string();
                        let is_optional = field_type_str.starts_with("Option <")
                            || field_type_str.starts_with("Option<");
                        if is_optional {
                            quote! {
                                if target_type == std::any::TypeId::of::<#target_ident>() {
                                    self.#field_name = Some(key as _);
                                }
                            }
                        } else {
                            quote! {
                                if target_type == std::any::TypeId::of::<#target_ident>() {
                                    self.#field_name = key as _;
                                }
                            }
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

            // G3.1: Emit `FIELD_TYPE_<NAME>` — the Rust type name as a `&str`
            // for introspection (debugging, typed projection validation).
            let type_const_name = syn::Ident::new(
                &format!("FIELD_TYPE_{}", field_name.to_string().to_uppercase()),
                field_name.span(),
            );
            column_consts.push(quote! {
                pub const #type_const_name: &'static str = stringify!(#field_ty);
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

            // G5: Emit `NAV_RELATED_<NAME>` — related entity type name for subquery resolution.
            let nav_info = detect_navigation_type(field_ty);
            let related_type_name = type_ident_string(&nav_info.related);
            let nav_related_const = syn::Ident::new(
                &format!("NAV_RELATED_{}", name_str.to_uppercase()),
                field_name.span(),
            );
            nav_field_consts.push(quote! {
                pub const #nav_related_const: &'static str = #related_type_name;
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

    // Generate the `set_auto_increment_key` body: assign the key to the
    // auto-increment PK field when present, no-op otherwise.
    let set_auto_inc_key_impl = match auto_inc_pk_ident {
        Some(ident) => quote! { self.#ident = key as _; },
        None => quote! { let _ = key; },
    };

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
                    property_index: std::sync::OnceLock::new(),
                    navigation_index: std::sync::OnceLock::new(),
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

            fn set_auto_increment_key(&mut self, key: i64) {
                #set_auto_inc_key_impl
            }

            fn set_foreign_key(&mut self, target_type: std::any::TypeId, key: i64) {
                #( #set_fk_arms )*
                let _ = (target_type, key);
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
            fn from_row(values: &[rust_ef::provider::DbValue]) -> rust_ef::error::EFResult<Self> {
                if values.len() < #field_count {
                    return Err(rust_ef::error::EFError::type_conversion(
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
                rows: &[Vec<rust_ef::provider::DbValue>],
            ) -> rust_ef::error::EFResult<()> {
                #( #has_many_setter_arms )*
                Ok(())
            }

            fn apply_reference(
                &mut self,
                field: &str,
                row: &[rust_ef::provider::DbValue],
            ) -> rust_ef::error::EFResult<()> {
                #( #reference_setter_arms )*
                Ok(())
            }

            fn drain_has_many(
                &mut self,
                field: &str,
            ) -> ::core::option::Option<Vec<Box<dyn std::any::Any + Send + Sync>>> {
                #( #drain_has_many_arms )*
                ::core::option::Option::None
            }

            async fn load_nested_includes(
                entities: &mut [Self],
                parent_navigation: &str,
                nested: &[rust_ef::query::IncludePath],
                provider: &dyn rust_ef::provider::IDatabaseProvider,
                filter_map: ::core::option::Option<&std::collections::HashMap<String, rust_ef::query::CompiledFilter>>,
            ) -> rust_ef::error::EFResult<()> {
                #( #nested_loader_arms )*
                Ok(())
            }
        }

        impl rust_ef::entity::ILazyInit for #struct_name {
            fn attach_lazy_contexts(
                &mut self,
                provider: std::sync::Arc<dyn rust_ef::provider::IDatabaseProvider>,
                filter_map: ::core::option::Option<std::sync::Arc<std::collections::HashMap<String, rust_ef::query::CompiledFilter>>>,
                depth: usize,
            ) {
                let meta = <Self as rust_ef::entity::IEntityType>::entity_meta();
                let key_values = <Self as rust_ef::entity::IGetKeyValues>::key_values(self);
                let snapshot = <Self as rust_ef::entity::IEntitySnapshot>::snapshot(self);
                #( #lazy_init_arms )*
            }
        }

        rust_ef::inventory::submit!({
            rust_ef::registration::EntityRegistration {
                type_id: std::any::TypeId::of::<#struct_name>(),
                type_name: stringify!(#struct_name),
                meta_fn: <#struct_name as rust_ef::entity::IEntityType>::entity_meta,
                context_key: #context_key_tokens,
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

/// Extracts the optional `#[context("key")]` attribute for multi-database
/// scenarios. Returns `None` for the default context, `Some("key")` for a
/// keyed context.
fn extract_context_key(attrs: &[syn::Attribute]) -> Option<proc_macro2::TokenStream> {
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

fn extract_sequence_name(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("sequence") {
            if let Ok(lit_str) = attr.parse_args::<LitStr>() {
                return Some(lit_str.value());
            }
        }
    }
    None
}

fn extract_on_delete(attrs: &[syn::Attribute]) -> proc_macro2::TokenStream {
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
