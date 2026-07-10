//! Field parsing — iterates struct fields and builds `EntityContext`.

use quote::quote;
use syn::spanned::Spanned;

use super::helpers::{
    detect_navigation_type, extract_column_name, extract_foreign_key_field_name,
    extract_foreign_key_target, extract_max_length, extract_on_delete, extract_sequence_name,
    extract_through_type, has_attr, is_navigation_field, type_ident_string, NavigationDiscriminant,
};

/// Holds all parsed field data needed by `gen::generate_impls`.
///
/// Built by `parse_fields`; consumed by `generate_impls`. The lifetime `'a`
/// is tied to the `DeriveInput`'s `Punctuated<Field, Comma>` — field ident
/// and type references are borrowed, not cloned.
pub(super) struct EntityContext<'a> {
    pub(super) struct_name: &'a syn::Ident,
    pub(super) struct_name_str: String,
    pub(super) table_name: String,
    pub(super) property_builders: Vec<proc_macro2::TokenStream>,
    pub(super) navigation_builders: Vec<proc_macro2::TokenStream>,
    pub(super) primary_key_names: Vec<proc_macro2::TokenStream>,
    pub(super) pk_column_names: Vec<String>,
    pub(super) fk_column_names: Vec<String>,
    pub(super) from_row_fields: Vec<(&'a syn::Ident, &'a syn::Type)>,
    pub(super) nav_field_names: Vec<&'a syn::Ident>,
    pub(super) lazy_init_arms: Vec<proc_macro2::TokenStream>,
    pub(super) pk_field_idents: Vec<&'a syn::Ident>,
    pub(super) has_many_setter_arms: Vec<proc_macro2::TokenStream>,
    pub(super) reference_setter_arms: Vec<proc_macro2::TokenStream>,
    pub(super) nested_loader_arms: Vec<proc_macro2::TokenStream>,
    pub(super) drain_has_many_arms: Vec<proc_macro2::TokenStream>,
    pub(super) fk_const_decls: Vec<proc_macro2::TokenStream>,
    pub(super) fk_index_arms: Vec<proc_macro2::TokenStream>,
    pub(super) fk_target_arms: Vec<proc_macro2::TokenStream>,
    pub(super) set_fk_arms: Vec<proc_macro2::TokenStream>,
    pub(super) pk_column_name_lit: proc_macro2::TokenStream,
    pub(super) pk_column_index_lit: proc_macro2::TokenStream,
    pub(super) auto_inc_pk_ident: Option<&'a syn::Ident>,
}

pub(super) fn parse_fields<'a>(
    struct_name: &'a syn::Ident,
    table_name: &str,
    fields: &'a syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
) -> syn::Result<EntityContext<'a>> {
    let struct_name_str = struct_name.to_string();

    let mut ctx = EntityContext {
        struct_name,
        struct_name_str,
        table_name: table_name.to_string(),
        property_builders: Vec::new(),
        navigation_builders: Vec::new(),
        primary_key_names: Vec::new(),
        pk_column_names: Vec::new(),
        fk_column_names: Vec::new(),
        from_row_fields: Vec::new(),
        nav_field_names: Vec::new(),
        lazy_init_arms: Vec::new(),
        pk_field_idents: Vec::new(),
        has_many_setter_arms: Vec::new(),
        reference_setter_arms: Vec::new(),
        nested_loader_arms: Vec::new(),
        drain_has_many_arms: Vec::new(),
        fk_const_decls: Vec::new(),
        fk_index_arms: Vec::new(),
        fk_target_arms: Vec::new(),
        set_fk_arms: Vec::new(),
        pk_column_name_lit: quote! { "id" },
        pk_column_index_lit: quote! { 0usize },
        auto_inc_pk_ident: None,
    };

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
            return Err(syn::Error::new(
                field.span(),
                "#[auto_increment] and #[sequence] are mutually exclusive",
            ));
        }
        let is_required = has_attr(&field.attrs, "required");
        let is_foreign_key = has_attr(&field.attrs, "foreign_key");
        let is_concurrency_token = has_attr(&field.attrs, "concurrency_check");
        let is_unique = has_attr(&field.attrs, "unique");
        let has_index = has_attr(&field.attrs, "index");
        let max_length = extract_max_length(&field.attrs);
        let column_name = extract_column_name(&field.attrs, &field_name_str);

        if is_primary_key {
            ctx.primary_key_names
                .push(quote! { std::borrow::Cow::Borrowed(#field_name_str) });
            ctx.pk_field_idents.push(field_name);
            ctx.pk_column_names.push(column_name.clone());
            ctx.pk_column_name_lit = quote! { #column_name };
            if is_auto_increment || is_sequence {
                ctx.auto_inc_pk_ident = Some(field_name);
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
            let parent_pk_col = ctx
                .pk_column_names
                .first()
                .map(|s| s.as_str())
                .unwrap_or("id");
            let parent_fk_col = ctx
                .fk_column_names
                .first()
                .map(|s| s.as_str())
                .unwrap_or("id");
            let parent_type_name = ctx.struct_name.to_string();
            let related_type_name = type_ident_string(inner_type);
            let fk_const =
                syn::Ident::new(&format!("FK_{}", parent_type_name), ctx.struct_name.span());
            let delete_behavior_tokens = extract_on_delete(&field.attrs);

            ctx.navigation_builders.push(match nav_kind {
                NavigationDiscriminant::ManyToMany => {
                    let join_type = nav_info
                        .join
                        .as_ref()
                        .expect("ManyToMany requires join type");
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
                    ctx.has_many_setter_arms.push(quote! {
                        if field == #field_name_str {
                            let items: rust_ef::error::EFResult<Vec<#inner_type>> = rows
                                .iter()
                                .map(|r| <#inner_type as rust_ef::entity::IFromRow>::from_row(r))
                                .collect();
                            self.#field_name = rust_ef::relations::HasMany::with(items?);
                            return Ok(());
                        }
                    });
                    ctx.drain_has_many_arms.push(quote! {
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
                    ctx.nested_loader_arms.push(quote! {
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
                    ctx.reference_setter_arms.push(quote! {
                        if field == #field_name_str {
                            self.#field_name = rust_ef::relations::BelongsTo::with(
                                <#inner_type as rust_ef::entity::IFromRow>::from_row(row)?,
                            );
                            return Ok(());
                        }
                    });
                    ctx.nested_loader_arms.push(quote! {
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
                    ctx.reference_setter_arms.push(quote! {
                        if field == #field_name_str {
                            self.#field_name = rust_ef::relations::HasOne::with(
                                <#inner_type as rust_ef::entity::IFromRow>::from_row(row)?,
                            );
                            return Ok(());
                        }
                    });
                    ctx.nested_loader_arms.push(quote! {
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
            ctx.nav_field_names.push(field_name);
            // Generate the lazy-init arm: looks up this navigation's meta
            // and attaches a LazyContextImpl to the container.
            ctx.lazy_init_arms.push(quote! {
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
            let scalar_idx = ctx.from_row_fields.len();
            if is_primary_key {
                ctx.pk_column_index_lit = quote! { #scalar_idx };
            }
            let sequence_name_lit = match &sequence_name {
                Some(name) => quote! { Some(std::borrow::Cow::Borrowed(#name)) },
                None => quote! { None },
            };
            ctx.property_builders.push(quote! {
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
            ctx.from_row_fields.push((field_name, field_type));
            if is_foreign_key {
                ctx.fk_column_names.push(column_name.clone());
                if let Some(target) = extract_foreign_key_target(&field.attrs) {
                    let target_ident = syn::Ident::new(&target, field_name.span());
                    let fk_ident = syn::Ident::new(&format!("FK_{}", target), field_name.span());
                    let col = column_name.clone();
                    ctx.fk_const_decls.push(quote! {
                        #[allow(non_upper_case_globals)]
                        pub const #fk_ident: &'static str = #col;
                    });
                    ctx.fk_index_arms.push(quote! {
                        #col | stringify!(#fk_ident) => #scalar_idx,
                    });
                    ctx.fk_target_arms.push(quote! {
                        if target == std::any::TypeId::of::<#target_ident>() {
                            return Some(#col);
                        }
                    });
                    ctx.set_fk_arms.push({
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

    Ok(ctx)
}
