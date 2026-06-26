//! `#[entity_config(T)]` attribute macro for `impl IEntityTypeConfiguration<T>` blocks.
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
//! #[entity_config(Blog)]
//! impl IEntityTypeConfiguration<Blog> for BlogConfig {
//!     fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
//!         entity.to_table("blogs_renamed");
//!     }
//! }
//! ```
//!
//! The attribute argument is the **entity type** (`Blog`), not the config
//! type. The config type is taken from the `impl`'s `Self` type
//! (`BlogConfig`). The closure stored in `EntityConfigRegistration::apply_fn`
//! is coercion-convertible to `fn(&mut ModelBuilder)` because it captures no
//! environment variables — only function pointers and `Default::default()`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, ItemImpl, Type};

pub fn expand_entity_config(args: TokenStream, input: TokenStream) -> TokenStream {
    let entity_ty: Type = parse_macro_input!(args as Type);
    let item_impl: ItemImpl = parse_macro_input!(input as ItemImpl);

    match rewrite_impl(&item_impl, &entity_ty) {
        Ok(tokens) => TokenStream::from(tokens),
        Err(err) => err.to_compile_error().into(),
    }
}

fn rewrite_impl(item: &ItemImpl, entity_ty: &Type) -> syn::Result<TokenStream2> {
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
            }
        });
    })
}
