//! Implements the `column!()` proc macro.
//!
//! Usage: `column!(Blog::url)` expands to `Blog::COLUMN_URL`
//!
//! Parses the input as a path like `TypeName::field_name` and
//! converts it to `TypeName::COLUMN_FIELD_NAME`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Expr, ExprField, Member};

pub fn expand_column(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as Expr);

    match &input {
        Expr::Field(ExprField { base, member, .. }) => {
            let ty = &**base;
            if let Member::Named(field_name) = member {
                let const_name = syn::Ident::new(
                    &format!("COLUMN_{}", field_name.to_string().to_uppercase()),
                    field_name.span(),
                );
                let expanded = quote! {
                    #ty::#const_name
                };
                return TokenStream::from(expanded);
            }
        }
        Expr::Path(path) if path.path.segments.len() >= 2 => {
            let segments = &path.path.segments;
            let type_part = &segments[0].ident;
            let field_part = &segments[1].ident;
            let const_name = syn::Ident::new(
                &format!("COLUMN_{}", field_part.to_string().to_uppercase()),
                field_part.span(),
            );
            let expanded = quote! {
                #type_part::#const_name
            };
            return TokenStream::from(expanded);
        }
        _ => {}
    }

    syn::Error::new_spanned(
        input,
        "expected `Type::field` syntax, e.g. `column!(Blog::url)`",
    )
    .to_compile_error()
    .into()
}
