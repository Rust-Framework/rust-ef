//! Impl generation — produces all trait impls from `EntityContext`.

use quote::quote;

use super::helpers::{
    detect_navigation_type, extract_column_name, generate_parse_expr, has_attr,
    is_navigation_field, type_ident_string,
};
use super::parse::EntityContext;

pub(super) fn generate_impls(
    ctx: &EntityContext<'_>,
    fields: &syn::punctuated::Punctuated<syn::Field, syn::token::Comma>,
    context_key_tokens: proc_macro2::TokenStream,
    is_view: bool,
    is_keyless: bool,
    table_comment: Option<String>,
    ef: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let struct_name = ctx.struct_name;
    let type_name_str = &ctx.struct_name_str;
    let table_name = &ctx.table_name;
    let field_count = ctx.from_row_fields.len();
    let table_comment_lit = match &table_comment {
        Some(c) => quote! { Some(std::borrow::Cow::Borrowed(#c)) },
        None => quote! { None },
    };

    // Generate FromRow field parsers for scalar fields
    let mut from_row_assignments = Vec::new();
    for (idx, (field_name, field_type)) in ctx.from_row_fields.iter().enumerate() {
        let idx_lit = syn::Index::from(idx);
        let type_str = quote!(#field_type).to_string();
        let parse_expr = generate_parse_expr(field_type, &type_str, idx_lit, ef);
        from_row_assignments.push(quote! {
            #field_name: #parse_expr,
        });
    }

    // EF-6: name-based materialization. Resolve every property by its column
    // name (a compile-time literal) into a positional row, then reuse the
    // positional parser. Reordered columns / views therefore cannot shift
    // values into the wrong field.
    let mut from_row_named_entries = Vec::new();
    for column_name in &ctx.from_row_column_names {
        from_row_named_entries.push(quote! {
            columns
                .iter()
                .find(|(name, _)| *name == #column_name)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| {
                    #ef::error::EFError::type_conversion(format!(
                        "column `{}` is missing from the named row for entity `{}`",
                        #column_name, #type_name_str
                    ))
                })?,
        });
    }

    // Generate column name constants for all mapped scalar fields
    let mut column_consts = Vec::new();
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
    for field_name in &ctx.nav_field_names {
        from_row_assignments.push(quote! {
            #field_name: Default::default(),
        });
    }

    // Build snapshot assignments: (field_name, DbValue::from(self.field))
    let mut snapshot_entries = Vec::new();
    for (field_name, _field_type) in &ctx.from_row_fields {
        let field_name_str = field_name.to_string();
        snapshot_entries.push(quote! {
            (#field_name_str, #ef::provider::DbValue::from(self.#field_name.clone())),
        });
    }

    // Generate the `set_auto_increment_key` body: assign the key to the
    // auto-increment PK field when present, no-op otherwise.
    let set_auto_inc_key_impl = match ctx.auto_inc_pk_ident {
        Some(ident) => quote! { self.#ident = key as _; },
        None => quote! { let _ = key; },
    };

    let property_builders = &ctx.property_builders;
    let navigation_builders = &ctx.navigation_builders;
    let foreign_key_builders = &ctx.foreign_key_builders;
    let primary_key_names = &ctx.primary_key_names;
    let fk_const_decls = &ctx.fk_const_decls;
    let fk_index_arms = &ctx.fk_index_arms;
    let fk_target_arms = &ctx.fk_target_arms;
    let pk_column_name_lit = &ctx.pk_column_name_lit;
    let pk_column_index_lit = &ctx.pk_column_index_lit;
    let pk_field_idents = &ctx.pk_field_idents;
    let set_fk_arms = &ctx.set_fk_arms;
    let has_many_setter_arms = &ctx.has_many_setter_arms;
    let reference_setter_arms = &ctx.reference_setter_arms;
    let drain_has_many_arms = &ctx.drain_has_many_arms;
    let nested_loader_arms = &ctx.nested_loader_arms;
    let lazy_init_arms = &ctx.lazy_init_arms;

    quote! {
        impl #ef::entity::IEntityType for #struct_name {
            fn entity_meta() -> #ef::metadata::EntityTypeMeta {
                #ef::metadata::EntityTypeMeta {
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
                    indexes: vec![],
                    foreign_keys: vec![
                        #(#foreign_key_builders,)*
                    ],
                    checks: vec![],
                    table_options: #ef::metadata::TableOptions::default(),
                    is_view: #is_view,
                    is_keyless: #is_keyless,
                    comment: #table_comment_lit,
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

        impl #ef::entity::IGetKeyValues for #struct_name {
            fn key_values(&self) -> #ef::entity_snapshot::EntitySnapshot {
                #ef::entity_snapshot::EntitySnapshot::new(vec![
                    #(
                        (stringify!(#pk_field_idents), #ef::provider::DbValue::from(self.#pk_field_idents.clone())),
                    )*
                ])
            }

            fn set_auto_increment_key(&mut self, key: i64) {
                #set_auto_inc_key_impl
            }

            fn set_foreign_key(&mut self, target_type: std::any::TypeId, key: i64) {
                #( #set_fk_arms )*
                let _ = (target_type, key);
            }
        }

        impl #ef::entity::IEntitySnapshot for #struct_name {
            fn snapshot(&self) -> #ef::entity_snapshot::EntitySnapshot {
                #ef::entity_snapshot::EntitySnapshot::new(vec![
                    #(#snapshot_entries)*
                ])
            }
        }

        impl #ef::entity::IFromRow for #struct_name {
            fn from_row(values: &[#ef::provider::DbValue]) -> #ef::error::EFResult<Self> {
                if values.len() < #field_count {
                    return Err(#ef::error::EFError::type_conversion(
                        format!("Expected {} columns, got {}", #field_count, values.len())
                    ));
                }
                Ok(#struct_name {
                    #(#from_row_assignments)*
                })
            }

            /// EF-6: materializes by column name, so column reordering cannot
            /// silently mis-assign values.
            fn from_row_named(
                columns: &[(&str, #ef::provider::DbValue)],
            ) -> #ef::error::EFResult<Self> {
                let values: Vec<#ef::provider::DbValue> = vec![
                    #(#from_row_named_entries)*
                ];
                <Self as #ef::entity::IFromRow>::from_row(&values)
            }
        }

        #[#ef::async_trait::async_trait]
        impl #ef::entity::INavigationSetter for #struct_name {
            fn apply_has_many(
                &mut self,
                field: &str,
                rows: &[Vec<#ef::provider::DbValue>],
            ) -> #ef::error::EFResult<()> {
                #( #has_many_setter_arms )*
                Ok(())
            }

            fn apply_reference(
                &mut self,
                field: &str,
                row: &[#ef::provider::DbValue],
            ) -> #ef::error::EFResult<()> {
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
                nested: &[#ef::query::IncludePath],
                provider: &dyn #ef::provider::IDatabaseProvider,
                filter_map: ::core::option::Option<&std::collections::HashMap<String, #ef::query::CompiledFilter>>,
            ) -> #ef::error::EFResult<()> {
                #( #nested_loader_arms )*
                Ok(())
            }
        }

        impl #ef::entity::ILazyInit for #struct_name {
            fn attach_lazy_contexts(
                &mut self,
                provider: std::sync::Arc<dyn #ef::provider::IDatabaseProvider>,
                filter_map: ::core::option::Option<std::sync::Arc<std::collections::HashMap<String, #ef::query::CompiledFilter>>>,
                depth: usize,
            ) {
                let meta = <Self as #ef::entity::IEntityType>::entity_meta();
                let key_values = <Self as #ef::entity::IGetKeyValues>::key_values(self);
                let snapshot = <Self as #ef::entity::IEntitySnapshot>::snapshot(self);
                #( #lazy_init_arms )*
            }
        }

        #ef::inventory::submit!({
            #ef::registration::EntityRegistration {
                type_id: std::any::TypeId::of::<#struct_name>(),
                type_name: stringify!(#struct_name),
                meta_fn: <#struct_name as #ef::entity::IEntityType>::entity_meta,
                context_key: #context_key_tokens,
            }
        });
    }
}
