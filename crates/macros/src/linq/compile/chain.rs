//! `=> field` order-clause codegen for the `linq!` macro.
//!
//! Boolean conditions are not compiled to method chains any more: `expand.rs`
//! turns the where body into a self-contained `BoolExpr` (see `bool.rs`) and
//! appends it with `QueryBuilder::where_`. That leaves this module with the
//! `=> order` sugar only.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::Expr;

use super::super::context::{extract_field, LinqCtx};

pub(crate) fn compile_order(
    ctx: &LinqCtx<'_>,
    expr: &Expr,
    descending: bool,
) -> syn::Result<TokenStream2> {
    let column = extract_field(ctx, expr)?;
    if descending {
        Ok(quote! { .order_by_desc_column(#column) })
    } else {
        Ok(quote! { .order_by_column(#column) })
    }
}
