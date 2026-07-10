//! G5 + v1.1: Subquery compilation — `EXISTS (...)` and `IN (SELECT ...)`.
//!
//! - `any` / `none` / `all` navigation methods → `EXISTS` / `NOT EXISTS`
//! - `in_subquery` → `IN (SELECT ...)` / `NOT IN (SELECT ...)`

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Expr, ExprClosure, ExprMethodCall, Ident, Pat, Type};

use super::super::context::{extract_field, extract_field_ref, field_const, FieldKind, LinqCtx};
use super::bool::compile_bool_expr;

/// Subquery quantifier kind for `any` / `none` / `all` navigation methods.
#[derive(Clone, Copy)]
pub(crate) enum SubqueryKind {
    /// `any` → `EXISTS (...)`
    Any,
    /// `none` → `NOT EXISTS (...)`
    None,
    /// `all` → `NOT EXISTS (... NOT <predicate>)`
    All,
}

impl SubqueryKind {
    /// Returns `true` when the subquery should be negated (`NOT EXISTS`).
    fn negated(self) -> bool {
        matches!(self, SubqueryKind::None | SubqueryKind::All)
    }

    /// Returns `true` when the predicate itself should be wrapped in `NOT`.
    fn negate_predicate(self) -> bool {
        matches!(self, SubqueryKind::All)
    }
}

/// Extracts the closure parameter identifier and its type annotation from a
/// subquery predicate closure `|p: Post| ...`. The type annotation is required
/// for subquery predicates (the related entity type cannot be inferred from
/// the navigation field at macro expansion time).
pub(crate) fn extract_subquery_closure(closure: &ExprClosure) -> syn::Result<(Ident, Type)> {
    let input = closure.inputs.first().ok_or_else(|| {
        syn::Error::new_spanned(closure, "subquery closure requires one parameter")
    })?;
    let pat_type = match input {
        Pat::Type(pt) => pt,
        _ => {
            return Err(syn::Error::new_spanned(
                input,
                "subquery closure parameter requires a type annotation (e.g. `|p: Post| ...`)",
            ));
        }
    };
    let param = match &*pat_type.pat {
        Pat::Ident(pi) => pi.ident.clone(),
        _ => {
            return Err(syn::Error::new_spanned(
                &pat_type.pat,
                "subquery closure parameter must be a simple identifier",
            ));
        }
    };
    Ok((param, (*pat_type.ty).clone()))
}

/// Shared helper: extracts navigation field constants and compiles the
/// predicate body into a `BoolExpr` value token stream.
///
/// Returns `(nav_field_const, nav_related_const, predicate_bool_expr)`.
fn compile_subquery_parts(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    kind: SubqueryKind,
) -> syn::Result<(TokenStream2, TokenStream2, TokenStream2)> {
    // Extract navigation field from receiver (e.g. `b.posts` → Blog, "posts")
    let nav_ref = extract_field_ref(ctx, &call.receiver)?;

    // `Entity::FIELD_<NAV>` — navigation field name constant
    let nav_field_const = field_const(&nav_ref.entity, &nav_ref.field_name, FieldKind::Navigation);

    // `Entity::NAV_RELATED_<NAV>` — related entity type name constant
    let nav_related_ident = Ident::new(
        &format!("NAV_RELATED_{}", nav_ref.field_name.to_uppercase()),
        proc_macro2::Span::call_site(),
    );
    let nav_entity = &nav_ref.entity;
    let nav_related_const = quote! { #nav_entity::#nav_related_ident };

    // Parse the closure argument `|p: Post| p.published`
    let closure_arg = call.args.first().ok_or_else(|| {
        syn::Error::new_spanned(
            call,
            "subquery method requires a closure argument like `|p: Post| p.published`",
        )
    })?;
    let closure = match closure_arg {
        Expr::Closure(c) => c,
        _ => {
            return Err(syn::Error::new_spanned(
                closure_arg,
                "subquery method requires a closure argument like `|p: Post| p.published`",
            ));
        }
    };

    // Extract param + related entity type from the closure
    let (param, related_entity) = extract_subquery_closure(closure)?;

    // Compile the predicate body in the related entity's context
    let sub_ctx = LinqCtx::single(&related_entity, Some(&param));
    let predicate_expr = compile_bool_expr(&sub_ctx, &closure.body)?;

    // Wrap in NOT for `all` (all p.X = NOT EXISTS any NOT p.X)
    let predicate_bool_expr = if kind.negate_predicate() {
        quote! {
            rust_ef::query::BoolExpr::Not(Box::new(#predicate_expr))
        }
    } else {
        predicate_expr
    };

    Ok((nav_field_const, nav_related_const, predicate_bool_expr))
}

/// Compiles `b.posts.any(|p: Post| p.published)` in the **method chain**
/// context (Form A/B where body) → `.where_exists_internal(...)`.
pub(super) fn compile_subquery_method(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    kind: SubqueryKind,
) -> syn::Result<TokenStream2> {
    let (nav_field, nav_related, predicate) = compile_subquery_parts(ctx, call, kind)?;
    let negated = kind.negated();
    Ok(quote! {
        .where_exists_internal(#nav_field, #nav_related, Some(#predicate), #negated)
    })
}

/// Compiles `b.posts.any(|p: Post| p.published)` in the **bool expression**
/// context (Form C filter, `&&`/`||` combinators) → `BoolExpr::Exists(...)`.
pub(crate) fn compile_subquery_bool(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    kind: SubqueryKind,
) -> syn::Result<TokenStream2> {
    let (nav_field, nav_related, predicate) = compile_subquery_parts(ctx, call, kind)?;
    let negated = kind.negated();
    let ctor = if negated {
        quote! { rust_ef::query::BoolExpr::NotExists }
    } else {
        quote! { rust_ef::query::BoolExpr::Exists }
    };
    Ok(quote! {
        #ctor(Box::new({
            let mut __spec = rust_ef::query::SubquerySpec::new(#nav_field, #nav_related);
            __spec.predicate = Some(Box::new(#predicate));
            __spec
        }))
    })
}

/// Compiles `!b.posts.any(...)` / `!none(...)` / `!all(...)` in the method
/// chain context by inverting the subquery quantifier.
///
/// - `!any` → `none` (NOT EXISTS, predicate not negated)
/// - `!none` → `any` (EXISTS, predicate not negated)
/// - `!all` → EXISTS with negated predicate (flip outer negation of `All`)
pub(super) fn compile_not_subquery(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
) -> syn::Result<TokenStream2> {
    let method_str = call.method.to_string();
    let (kind, flip_outer) = match method_str.as_str() {
        "any" => (SubqueryKind::None, false),
        "none" => (SubqueryKind::Any, false),
        // `All` = NOT EXISTS(NOT pred); `!all` = EXISTS(NOT pred) → flip outer.
        "all" => (SubqueryKind::All, true),
        _ => unreachable!("compile_not_subquery called on non-subquery method"),
    };
    let (nav_field, nav_related, predicate) = compile_subquery_parts(ctx, call, kind)?;
    let negated = kind.negated() ^ flip_outer;
    Ok(quote! {
        .where_exists_internal(#nav_field, #nav_related, Some(#predicate), #negated)
    })
}

// ---------------------------------------------------------------------------
// v1.1: IN (SELECT ...) / NOT IN (SELECT ...) subquery support
// ---------------------------------------------------------------------------

/// Extracts the outer column, source table, and projection column from an
/// `in_subquery` closure call.
///
/// Syntax: `b.field.in_subquery(|p: Post| p.blog_id)`
/// - Receiver `b.field` → outer column (`Blog::COLUMN_FIELD`)
/// - Closure param type `Post` → source table (`Post::TABLE`)
/// - Closure body `p.blog_id` → projection column (`Post::COLUMN_BLOG_ID`)
fn compile_in_subquery_parts(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
) -> syn::Result<(TokenStream2, TokenStream2, TokenStream2)> {
    // Outer column from receiver `b.field`
    let outer_column = extract_field(ctx, &call.receiver)?;

    // Parse closure `|p: Post| p.blog_id`
    let closure_arg = call.args.first().ok_or_else(|| {
        syn::Error::new_spanned(
            call,
            "in_subquery requires a closure like `|p: Post| p.blog_id`",
        )
    })?;
    let closure = match closure_arg {
        Expr::Closure(c) => c,
        _ => {
            return Err(syn::Error::new_spanned(
                closure_arg,
                "in_subquery requires a closure like `|p: Post| p.blog_id`",
            ));
        }
    };

    let (param, related_entity) = extract_subquery_closure(closure)?;

    // Projection column from closure body `p.blog_id`
    let sub_ctx = LinqCtx::single(&related_entity, Some(&param));
    let projection_column = extract_field(&sub_ctx, &closure.body)?;

    // Source table from related entity type
    Ok((
        outer_column,
        quote! { #related_entity::TABLE },
        projection_column,
    ))
}

/// Compiles `b.field.in_subquery(|p: Post| p.blog_id)` in the **method chain**
/// context (Form A/B where body) → `.where_in_subquery_internal(...)`.
pub(crate) fn compile_in_subquery_method(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    negated: bool,
) -> syn::Result<TokenStream2> {
    let (outer_col, source_tbl, proj_col) = compile_in_subquery_parts(ctx, call)?;
    Ok(quote! {
        .where_in_subquery_internal(#outer_col, #source_tbl, #proj_col, ::core::option::Option::None, #negated)
    })
}

/// Compiles `b.field.in_subquery(|p: Post| p.blog_id)` in the **bool
/// expression** context (Form C filter) → `BoolExpr::InSubquery(...)`.
pub(crate) fn compile_in_subquery_bool(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    negated: bool,
) -> syn::Result<TokenStream2> {
    let (outer_col, source_tbl, proj_col) = compile_in_subquery_parts(ctx, call)?;
    // Resolve the &'static str constants to owned Strings for the spec.
    let ctor = if negated {
        quote! { rust_ef::query::BoolExpr::NotInSubquery }
    } else {
        quote! { rust_ef::query::BoolExpr::InSubquery }
    };
    Ok(quote! {
        #ctor(Box::new(
            rust_ef::query::InSubquerySpec::new(#outer_col, #source_tbl, #proj_col)
        ))
    })
}
