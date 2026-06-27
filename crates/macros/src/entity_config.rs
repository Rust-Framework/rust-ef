//! `#[entity(T)]` attribute macro for `impl IEntityTypeConfiguration<T>` blocks.
//!
//! Emits an `inventory::submit!` registering an `EntityConfigRegistration`
//! whose `apply_fn` instantiates the configuration via `Default::default()`
//! and applies it to a `ModelBuilder` through `EntityTypeBuilder`.
//!
//! # Usage
//!
//! ```ignore
//! #[derive(Default)]
//! pub struct BlogConfig;
//!
//! // Default context
//! #[entity(Blog)]
//! impl IEntityTypeConfiguration<Blog> for BlogConfig {
//!     fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
//!         entity.to_table("blogs_renamed");
//!     }
//! }
//!
//! // Keyed context — config applies only to the "logs" DbContext
//! #[entity(LogEntry, "logs")]
//! impl IEntityTypeConfiguration<LogEntry> for LogEntryConfig {
//!     fn configure(&self, entity: &mut EntityTypeBuilder<'_, LogEntry>) {
//!         entity.to_table("app_logs");
//!     }
//! }
//! ```
//!
//! The first attribute argument is the **entity type** (`Blog`), not the
//! config type. The config type is taken from the `impl`'s `Self` type
//! (`BlogConfig`). The optional second argument is a string literal
//! specifying the DbContext key for multi-database scenarios. The closure
//! stored in `EntityConfigRegistration::apply_fn` is coercion-convertible to
//! `fn(&mut ModelBuilder)` because it captures no environment variables.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, ItemImpl, LitStr, Type};

pub fn expand_entity_config(args: TokenStream, input: TokenStream) -> TokenStream {
    // Parse: `Entity` or `Entity, "key"`
    let args_span = proc_macro2::Span::call_site();
    let parsed: EntityConfigArgs = match syn::parse::<EntityConfigArgs>(args) {
        Ok(p) => p,
        Err(_) => {
            return syn::Error::new(
                args_span,
                "expected `#[entity(EntityType)]` or `#[entity(EntityType, \"context_key\")]`",
            )
            .to_compile_error()
            .into();
        }
    };

    let entity_ty = parsed.entity_ty;
    let context_key_tokens = match parsed.context_key {
        Some(key) => quote! { ::core::option::Option::Some(#key) },
        None => quote! { ::core::option::Option::None },
    };

    let item_impl: ItemImpl = parse_macro_input!(input as ItemImpl);

    match rewrite_impl(&item_impl, &entity_ty, &context_key_tokens) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => err.to_compile_error().into(),
    }
}

struct EntityConfigArgs {
    entity_ty: Type,
    context_key: Option<LitStr>,
}

impl syn::parse::Parse for EntityConfigArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let entity_ty: Type = input.parse()?;
        let context_key = if input.peek(syn::Token![,]) {
            let _: syn::Token![,] = input.parse()?;
            Some(input.parse::<LitStr>()?)
        } else {
            None
        };
        Ok(EntityConfigArgs {
            entity_ty,
            context_key,
        })
    }
}

fn rewrite_impl(
    item: &ItemImpl,
    entity_ty: &Type,
    context_key_tokens: &TokenStream2,
) -> syn::Result<TokenStream2> {
    let self_ty = &item.self_ty;
    let impl_tokens = item.to_token_stream();

    Ok(quote! {
        #impl_tokens

        rust_ef::inventory::submit!({
            rust_ef::registration::EntityConfigRegistration {
                type_id: std::any::TypeId::of::<#entity_ty>(),
                type_name: stringify!(#entity_ty),
                apply_fn: |builder: &mut rust_ef::model_builder::ModelBuilder| {
                    let meta = <#entity_ty as rust_ef::entity::IEntityType>::entity_meta();
                    builder.register_entity_meta(meta);
                    let config = <#self_ty as std::default::Default>::default();
                    let type_id = std::any::TypeId::of::<#entity_ty>();
                    let mut entity_builder =
                        <rust_ef::model_builder::EntityTypeBuilder<'_, #entity_ty>>
                            ::new(builder, type_id);
                    <#self_ty as rust_ef::model_builder::IEntityTypeConfiguration<#entity_ty>>
                        ::configure(&config, &mut entity_builder);
                },
                context_key: #context_key_tokens,
            }
        });
    })
}
