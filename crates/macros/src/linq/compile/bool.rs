//! Form C filter → `BoolExpr` value compilation.
//!
//! Mirrors `compile_expr` but emits `BoolExpr::Filter(...)` / `BoolExpr::And(...)`
//! / `BoolExpr::Or(...)` / `BoolExpr::Not(...)` value expressions instead of
//! `QueryBuilder` method chains.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{BinOp, Expr, ExprMethodCall, ExprUnary, UnOp};

use super::super::context::{extract_field, extract_value, LinqCtx};
use super::subquery::{compile_in_subquery_bool, compile_subquery_bool, SubqueryKind};

/// Compiles a boolean expression to a `BoolExpr` value (Form C `filter`).
pub(crate) fn compile_bool_expr(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Group(g) => compile_bool_expr(ctx, &g.expr),
        Expr::Paren(p) => compile_bool_expr(ctx, &p.expr),
        Expr::Unary(ExprUnary {
            op: UnOp::Not(_),
            expr: inner,
            ..
        }) => {
            let inner_ts = compile_bool_expr(ctx, inner)?;
            Ok(quote! {
                rust_ef::query::BoolExpr::Not(Box::new(#inner_ts))
            })
        }
        Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
            let left = compile_bool_expr(ctx, &b.left)?;
            let right = compile_bool_expr(ctx, &b.right)?;
            Ok(quote! {
                rust_ef::query::BoolExpr::And(
                    Box::new(#left),
                    Box::new(#right),
                )
            })
        }
        Expr::Binary(b) if matches!(b.op, BinOp::Or(_)) => {
            let left = compile_bool_expr(ctx, &b.left)?;
            let right = compile_bool_expr(ctx, &b.right)?;
            Ok(quote! {
                rust_ef::query::BoolExpr::Or(
                    Box::new(#left),
                    Box::new(#right),
                )
            })
        }
        Expr::MethodCall(call) => compile_bool_method(ctx, call),
        Expr::Field(_) | Expr::Path(_) => {
            // Boolean field: `b.active` → `col = true`
            let column = extract_field(ctx, expr)?;
            Ok(quote! {
                rust_ef::query::BoolExpr::Filter(
                    rust_ef::query::FilterCondition::with_values(
                        #column, "=", vec![rust_ef::provider::DbValue::Bool(true)],
                    )
                )
            })
        }
        _ => compile_bool_comparison(ctx, expr),
    }
}

pub(crate) fn compile_bool_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
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
        rust_ef::query::BoolExpr::Filter(
            rust_ef::query::FilterCondition::with_values(
                #column, #op,
                vec![rust_ef::provider::DbValue::from(#value)],
            )
        )
    })
}

/// Compiles method-call boolean expressions for Form C (is_null, is_not_null, contains, etc.).
pub(crate) fn compile_bool_method(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
) -> syn::Result<TokenStream2> {
    let method = call.method.to_string();

    // G5: Subquery methods — `b.posts.any(|p: Post| p.published)` etc.
    match method.as_str() {
        "any" => return compile_subquery_bool(ctx, call, SubqueryKind::Any),
        "none" => return compile_subquery_bool(ctx, call, SubqueryKind::None),
        "all" => return compile_subquery_bool(ctx, call, SubqueryKind::All),
        // v1.1: IN (SELECT ...) subquery
        "in_subquery" => return compile_in_subquery_bool(ctx, call, false),
        _ => {}
    }

    let column = extract_field(ctx, &call.receiver)?;

    match method.as_str() {
        "is_null" if call.args.is_empty() => Ok(quote! {
            rust_ef::query::BoolExpr::Filter(
                rust_ef::query::FilterCondition::new(#column, "IS NULL", 0)
            )
        }),
        "is_not_null" if call.args.is_empty() => Ok(quote! {
            rust_ef::query::BoolExpr::Filter(
                rust_ef::query::FilterCondition::new(#column, "IS NOT NULL", 0)
            )
        }),
        "contains" => {
            let arg = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.contains()` requires one argument")
            })?;
            // If the argument is a field, it's IN (LINQ style); otherwise LIKE.
            if extract_field(ctx, arg).is_ok() {
                let values = extract_value(&call.receiver)?;
                Ok(quote! {
                    rust_ef::query::BoolExpr::Filter(
                        rust_ef::query::FilterCondition::with_values(
                            #column, "IN",
                            (#values).into_iter()
                                .map(rust_ef::provider::DbValue::from)
                                .collect::<Vec<_>>(),
                        )
                    )
                })
            } else {
                let value = extract_value(arg)?;
                Ok(quote! {
                    rust_ef::query::BoolExpr::Filter(
                        rust_ef::query::FilterCondition::with_values(
                            #column, "LIKE",
                            vec![rust_ef::provider::DbValue::from(
                                rust_ef::query::like_contains(#value)
                            )],
                        )
                    )
                })
            }
        }
        "starts_with" => {
            let arg = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.starts_with()` requires one argument")
            })?;
            let value = extract_value(arg)?;
            Ok(quote! {
                rust_ef::query::BoolExpr::Filter(
                    rust_ef::query::FilterCondition::with_values(
                        #column, "LIKE",
                        vec![rust_ef::provider::DbValue::from(
                            rust_ef::query::like_starts_with(#value)
                        )],
                    )
                )
            })
        }
        "ends_with" => {
            let arg = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.ends_with()` requires one argument")
            })?;
            let value = extract_value(arg)?;
            Ok(quote! {
                rust_ef::query::BoolExpr::Filter(
                    rust_ef::query::FilterCondition::with_values(
                        #column, "LIKE",
                        vec![rust_ef::provider::DbValue::from(
                            rust_ef::query::like_ends_with(#value)
                        )],
                    )
                )
            })
        }
        "between" => {
            let low = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.between()` requires low and high")
            })?;
            let high = call.args.get(1).ok_or_else(|| {
                syn::Error::new_spanned(call, "`.between()` requires low and high")
            })?;
            let lo = extract_value(low)?;
            let hi = extract_value(high)?;
            Ok(quote! {
                rust_ef::query::BoolExpr::Filter(
                    rust_ef::query::FilterCondition::with_values(
                        #column, "BETWEEN",
                        vec![
                            rust_ef::provider::DbValue::from(#lo),
                            rust_ef::provider::DbValue::from(#hi),
                        ],
                    )
                )
            })
        }
        _ => Err(syn::Error::new_spanned(
            &call.method,
            "supported methods: contains, starts_with, ends_with, is_null, is_not_null, between",
        )),
    }
}
