//! Form A/B where body → `QueryBuilder` method chain compilation.
//!
//! Also hosts `compile_method` (method-call dispatch shared with bool form)
//! and `compile_order` (order-by codegen).

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{BinOp, Expr, ExprMethodCall, ExprUnary, UnOp};

use super::super::context::{extract_field, extract_value, LinqCtx};
use super::subquery::{
    compile_in_subquery_method, compile_not_subquery, compile_subquery_method, SubqueryKind,
};

pub(crate) fn compile_expr(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Group(g) => compile_expr(ctx, &g.expr),
        Expr::Paren(p) => compile_expr(ctx, &p.expr),
        Expr::Unary(ExprUnary {
            op: UnOp::Not(_),
            expr: inner,
            ..
        }) => compile_not(ctx, inner),
        Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
            let left = compile_expr(ctx, &b.left)?;
            let right = compile_expr(ctx, &b.right)?;
            Ok(quote! { #left #right })
        }
        Expr::Binary(b) if matches!(b.op, BinOp::Or(_)) => {
            let left = compile_expr(ctx, &b.left)?;
            let right = compile_expr(ctx, &b.right)?;
            Ok(quote! { #left .or_where(|__sub| __sub #right) })
        }
        Expr::MethodCall(call) => compile_method(ctx, call),
        Expr::Field(_) | Expr::Path(_) => compile_bool_member(ctx, expr),
        _ => compile_comparison(ctx, expr),
    }
}

pub(crate) fn compile_not(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Group(g) => compile_not(ctx, &g.expr),
        Expr::Paren(p) => compile_not(ctx, &p.expr),
        Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
            let left = compile_not(ctx, &b.left)?;
            let right = compile_not(ctx, &b.right)?;
            Ok(quote! { #left .or_where(|__sub| __sub #right) })
        }
        Expr::Binary(b) if matches!(b.op, BinOp::Or(_)) => {
            let left = compile_not(ctx, &b.left)?;
            let right = compile_not(ctx, &b.right)?;
            Ok(quote! { #left #right })
        }
        Expr::Field(_) | Expr::Path(_) => {
            let column = extract_field(ctx, expr)?;
            Ok(quote! {
                .filter_column(#column, "=", rust_ef::provider::DbValue::Bool(false))
            })
        }
        // G5: `!b.posts.any(...)` = `none(...)`, `!none(...)` = `any(...)`,
        // `!all(p.X)` = `EXISTS(NOT p.X)`.
        Expr::MethodCall(call)
            if matches!(call.method.to_string().as_str(), "any" | "none" | "all") =>
        {
            compile_not_subquery(ctx, call)
        }
        Expr::MethodCall(call) if call.method == "contains" => compile_contains(ctx, call, true),
        // v1.1: `!b.field.in_subquery(...)` → NOT IN (SELECT ...)
        Expr::MethodCall(call) if call.method == "in_subquery" => {
            compile_in_subquery_method(ctx, call, true)
        }
        _ => compile_negated_comparison(ctx, expr),
    }
}

fn compile_bool_member(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    let column = extract_field(ctx, expr)?;
    Ok(quote! {
        .filter_column(#column, "=", rust_ef::provider::DbValue::Bool(true))
    })
}

pub(crate) fn compile_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    let binary = match expr {
        Expr::Binary(b) => b,
        _ => {
            return Err(syn::Error::new_spanned(
                expr,
                "expected comparison, e.g. `b.rating > 5`",
            ));
        }
    };

    let op = match binary.op {
        BinOp::Eq(_) => "=",
        BinOp::Ne(_) => "!=",
        BinOp::Gt(_) => ">",
        BinOp::Ge(_) => ">=",
        BinOp::Lt(_) => "<",
        BinOp::Le(_) => "<=",
        _ => {
            return Err(syn::Error::new_spanned(
                binary.op,
                "unsupported operator; use ==, !=, >, >=, <, <=",
            ));
        }
    };

    let column = extract_field(ctx, &binary.left)?;
    let value = extract_value(&binary.right)?;

    Ok(quote! {
        .filter_column(#column, #op, rust_ef::provider::DbValue::from(#value))
    })
}

pub(crate) fn compile_negated_comparison(
    ctx: &LinqCtx<'_>,
    expr: &Expr,
) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Group(g) => compile_negated_comparison(ctx, &g.expr),
        Expr::Paren(p) => compile_negated_comparison(ctx, &p.expr),
        Expr::MethodCall(call) if call.method == "contains" => compile_contains(ctx, call, true),
        Expr::Binary(b) => {
            let op = match b.op {
                BinOp::Eq(_) => "=",
                BinOp::Ne(_) => "!=",
                BinOp::Gt(_) => ">",
                BinOp::Ge(_) => ">=",
                BinOp::Lt(_) => "<",
                BinOp::Le(_) => "<=",
                _ => {
                    return Err(syn::Error::new_spanned(
                        b.op,
                        "unsupported operator inside NOT",
                    ));
                }
            };
            let column = extract_field(ctx, &b.left)?;
            let value = extract_value(&b.right)?;
            Ok(quote! {
                .filter_not(#column, #op, rust_ef::provider::DbValue::from(#value))
            })
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "NOT supports comparisons and `.contains()`",
        )),
    }
}

/// `b.url.contains("x")` → LIKE; `ids.contains(b.id)` → IN (LINQ style).
pub(crate) fn compile_contains(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    negate: bool,
) -> syn::Result<TokenStream2> {
    let arg = call
        .args
        .first()
        .ok_or_else(|| syn::Error::new_spanned(call, "`.contains()` requires one argument"))?;

    if let Ok(column) = extract_field(ctx, arg) {
        let values = extract_value(&call.receiver)?;
        let values_list = quote! {
            {
                use rust_ef::provider::DbValue;
                (#values).into_iter().map(DbValue::from).collect::<Vec<_>>()
            }
        };
        return Ok(if negate {
            quote! { .filter_not_in(#column, #values_list) }
        } else {
            quote! { .filter_in(#column, #values_list) }
        });
    }

    let column = extract_field(ctx, &call.receiver)?;
    let value = extract_value(arg)?;
    Ok(if negate {
        quote! { .filter_not_like(#column, rust_ef::query::like_contains(#value)) }
    } else {
        quote! { .filter_like(#column, rust_ef::query::like_contains(#value)) }
    })
}

pub(crate) fn compile_method(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
) -> syn::Result<TokenStream2> {
    let method = call.method.to_string();

    // G5: Subquery methods — `b.posts.any(|p: Post| p.published)` etc.
    match method.as_str() {
        "any" => return compile_subquery_method(ctx, call, SubqueryKind::Any),
        "none" => return compile_subquery_method(ctx, call, SubqueryKind::None),
        "all" => return compile_subquery_method(ctx, call, SubqueryKind::All),
        // v1.1: IN (SELECT ...) subquery
        "in_subquery" => return compile_in_subquery_method(ctx, call, false),
        _ => {}
    }

    if method == "contains" {
        return compile_contains(ctx, call, false);
    }

    let column = extract_field(ctx, &call.receiver)?;

    match method.as_str() {
        "starts_with" => {
            let arg = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.starts_with()` requires one argument")
            })?;
            let value = extract_value(arg)?;
            Ok(quote! {
                .filter_like(#column, rust_ef::query::like_starts_with(#value))
            })
        }
        "ends_with" => {
            let arg = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.ends_with()` requires one argument")
            })?;
            let value = extract_value(arg)?;
            Ok(quote! {
                .filter_like(#column, rust_ef::query::like_ends_with(#value))
            })
        }
        "is_null" if call.args.is_empty() => Ok(quote! { .filter_is_null(#column) }),
        "is_not_null" if call.args.is_empty() => Ok(quote! { .filter_is_not_null(#column) }),
        "between" => {
            let low = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.between()` requires low and high values")
            })?;
            let high = call.args.get(1).ok_or_else(|| {
                syn::Error::new_spanned(call, "`.between()` requires low and high values")
            })?;
            let lo = extract_value(low)?;
            let hi = extract_value(high)?;
            Ok(quote! {
                .filter_between(#column, rust_ef::provider::DbValue::from(#lo), rust_ef::provider::DbValue::from(#hi))
            })
        }
        _ => Err(syn::Error::new_spanned(
            &call.method,
            "supported methods: contains, starts_with, ends_with, is_null, is_not_null, between",
        )),
    }
}

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
