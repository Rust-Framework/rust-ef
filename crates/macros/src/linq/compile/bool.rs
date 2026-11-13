//! Condition expression → `BoolExpr` value compilation.
//!
//! Mirrors `compile_expr` but emits `BoolExpr::Filter(...)` / `BoolExpr::And(...)`
//! / `BoolExpr::Or(...)` / `BoolExpr::Not(...)` value expressions instead of
//! `QueryBuilder` method chains.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{BinOp, Expr, ExprMethodCall, ExprUnary, UnOp};

use super::super::context::{extract_field, extract_value, is_field_expr, LinqCtx};
use super::subquery::{compile_in_subquery_bool, compile_subquery_bool, SubqueryKind};

/// Compiles a boolean expression to a self-contained `BoolExpr` value.
pub(crate) fn compile_bool_expr(
    ctx: &LinqCtx<'_>,
    expr: &Expr,
    ef: &TokenStream2,
) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Group(g) => compile_bool_expr(ctx, &g.expr, ef),
        Expr::Paren(p) => compile_bool_expr(ctx, &p.expr, ef),
        Expr::Unary(ExprUnary {
            op: UnOp::Not(_),
            expr: inner,
            ..
        }) => {
            let inner_ts = compile_bool_expr(ctx, inner, ef)?;
            Ok(quote! {
                #ef::query::BoolExpr::Not(Box::new(#inner_ts))
            })
        }
        Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
            let left = compile_bool_expr(ctx, &b.left, ef)?;
            let right = compile_bool_expr(ctx, &b.right, ef)?;
            Ok(quote! {
                #ef::query::BoolExpr::And(
                    Box::new(#left),
                    Box::new(#right),
                )
            })
        }
        Expr::Binary(b) if matches!(b.op, BinOp::Or(_)) => {
            let left = compile_bool_expr(ctx, &b.left, ef)?;
            let right = compile_bool_expr(ctx, &b.right, ef)?;
            Ok(quote! {
                #ef::query::BoolExpr::Or(
                    Box::new(#left),
                    Box::new(#right),
                )
            })
        }
        Expr::MethodCall(call) => compile_bool_method(ctx, call, ef),
        Expr::Field(_) | Expr::Path(_) => {
            // Boolean field: `b.active` → `col = true`
            let column = extract_field(ctx, expr)?;
            Ok(quote! {
                #ef::query::BoolExpr::Filter(
                    #ef::query::FilterCondition::with_values(
                        #column, "=", vec![#ef::provider::DbValue::Bool(true)],
                    )
                )
            })
        }
        _ => compile_bool_comparison(ctx, expr, ef),
    }
}

pub(crate) fn compile_bool_comparison(
    ctx: &LinqCtx<'_>,
    expr: &Expr,
    ef: &TokenStream2,
) -> syn::Result<TokenStream2> {
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
        #ef::query::BoolExpr::Filter(
            #ef::query::FilterCondition::with_values(
                #column, #op,
                vec![#ef::provider::DbValue::from(#value)],
            )
        )
    })
}

/// Compiles method-call conditions (is_null, is_not_null, contains, ...).
pub(crate) fn compile_bool_method(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    ef: &TokenStream2,
) -> syn::Result<TokenStream2> {
    let method = call.method.to_string();

    // G5: Subquery methods — `b.posts.any(|p: Post| p.published)` etc.
    match method.as_str() {
        "any" => return compile_subquery_bool(ctx, call, SubqueryKind::Any, ef),
        "none" => return compile_subquery_bool(ctx, call, SubqueryKind::None, ef),
        "all" => return compile_subquery_bool(ctx, call, SubqueryKind::All, ef),
        // v1.1: IN (SELECT ...) subquery
        "in_subquery" => return compile_in_subquery_bool(ctx, call, false, ef),
        _ => {}
    }

    let column = extract_field(ctx, &call.receiver)?;

    match method.as_str() {
        "is_null" if call.args.is_empty() => Ok(quote! {
            #ef::query::BoolExpr::Filter(
                #ef::query::FilterCondition::new(#column, "IS NULL", 0)
            )
        }),
        "is_not_null" if call.args.is_empty() => Ok(quote! {
            #ef::query::BoolExpr::Filter(
                #ef::query::FilterCondition::new(#column, "IS NOT NULL", 0)
            )
        }),
        "contains" => {
            let arg = call.args.first().ok_or_else(|| {
                syn::Error::new_spanned(call, "`.contains()` requires one argument")
            })?;
            // If the argument is a field reference, it's IN (LINQ style);
            // otherwise it's a value, and `.contains()` means LIKE.
            if is_field_expr(arg) {
                // `ids.contains(b.id)`: the column is the argument, the values
                // are the receiver.
                let column = extract_field(ctx, arg)?;
                let values = extract_value(&call.receiver)?;
                Ok(quote! {
                    #ef::query::BoolExpr::Filter(
                        #ef::query::FilterCondition::with_values(
                            #column, "IN",
                            (#values).into_iter()
                                .map(#ef::provider::DbValue::from)
                                .collect::<Vec<_>>(),
                        )
                    )
                })
            } else {
                let value = extract_value(arg)?;
                Ok(quote! {
                    #ef::query::BoolExpr::Filter(
                        #ef::query::FilterCondition::with_values(
                            #column, "LIKE",
                            vec![#ef::provider::DbValue::from(
                                #ef::query::like_contains(#value)
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
                #ef::query::BoolExpr::Filter(
                    #ef::query::FilterCondition::with_values(
                        #column, "LIKE",
                        vec![#ef::provider::DbValue::from(
                            #ef::query::like_starts_with(#value)
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
                #ef::query::BoolExpr::Filter(
                    #ef::query::FilterCondition::with_values(
                        #column, "LIKE",
                        vec![#ef::provider::DbValue::from(
                            #ef::query::like_ends_with(#value)
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
                #ef::query::BoolExpr::Filter(
                    #ef::query::FilterCondition::with_values(
                        #column, "BETWEEN",
                        vec![
                            #ef::provider::DbValue::from(#lo),
                            #ef::provider::DbValue::from(#hi),
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
