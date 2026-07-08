//! Expression compilation for the `linq!` macro.
//!
//! Translates `syn::Expr` AST nodes into `TokenStream2` fragments that
//! construct either `QueryBuilder` method chains (Form A/B where body) or
//! `BoolExpr` value expressions (Form C filter, `&&`/`||` combinators).
//!
//! - `compile_expr` / `compile_method` / `compile_comparison` — method-chain form
//! - `compile_bool_expr` / `compile_bool_method` — `BoolExpr` value form
//! - `compile_subquery_*` / `compile_in_subquery_*` — EXISTS / IN (SELECT…) subqueries
//! - `compile_having_expr` — `HavingExpr` value for `having` clauses

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{BinOp, Expr, ExprClosure, ExprMethodCall, ExprUnary, Ident, Pat, Type, UnOp};

use super::ast::HavingExprAst;
use super::context::{
    extract_field, extract_field_ref, extract_value, field_const, FieldKind, LinqCtx,
};

// ---------------------------------------------------------------------------
// compile_bool_expr — Form C filter → BoolExpr value
// ---------------------------------------------------------------------------

/// Compiles a boolean expression to a `BoolExpr` value (Form C `filter`).
///
/// Mirrors `compile_expr` but emits `BoolExpr::Filter(...)` / `BoolExpr::And(...)`
/// / `BoolExpr::Or(...)` / `BoolExpr::Not(...)` value expressions instead of
/// `QueryBuilder` method chains.
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

// ---------------------------------------------------------------------------
// compile_expr — Form A/B where body → QueryBuilder method chain
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// G5: Subquery compilation — `b.posts.any(|p: Post| p.published)`
// ---------------------------------------------------------------------------

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
fn compile_subquery_method(
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
fn compile_subquery_bool(
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
fn compile_not_subquery(ctx: &LinqCtx<'_>, call: &ExprMethodCall) -> syn::Result<TokenStream2> {
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

// ---------------------------------------------------------------------------
// HAVING expression compilation
// ---------------------------------------------------------------------------

/// Compiles a `HavingExprAst` into Rust code that constructs the corresponding
/// `rust_ef::query::HavingExpr` runtime value.
///
/// Column references are resolved to `Entity::COLUMN_*` constants via
/// `extract_field`, and value literals via `extract_value`. Aggregate names
/// and operators (validated during parsing) are mapped to enum variants.
pub(crate) fn compile_having_expr(
    ast: &HavingExprAst,
    ctx: &LinqCtx<'_>,
) -> syn::Result<TokenStream2> {
    match ast {
        HavingExprAst::Compare {
            agg,
            col,
            op,
            value,
        } => {
            let col_const = extract_field(ctx, col)?;
            let val = extract_value(value)?;
            let agg_kind = agg_kind_ident(agg);
            let op_ident = op_to_ident(op);
            Ok(quote! {
                rust_ef::query::HavingExpr::Compare {
                    agg: rust_ef::query::AggKind::#agg_kind,
                    col: #col_const.to_string(),
                    op: rust_ef::query::CompareOp::#op_ident,
                    value: rust_ef::provider::DbValue::from(#val),
                }
            })
        }
        HavingExprAst::And(left, right) => {
            let l = compile_having_expr(left, ctx)?;
            let r = compile_having_expr(right, ctx)?;
            Ok(quote! {
                rust_ef::query::HavingExpr::And(Box::new(#l), Box::new(#r))
            })
        }
        HavingExprAst::Or(left, right) => {
            let l = compile_having_expr(left, ctx)?;
            let r = compile_having_expr(right, ctx)?;
            Ok(quote! {
                rust_ef::query::HavingExpr::Or(Box::new(#l), Box::new(#r))
            })
        }
        HavingExprAst::Not(inner) => {
            let i = compile_having_expr(inner, ctx)?;
            Ok(quote! {
                rust_ef::query::HavingExpr::Not(Box::new(#i))
            })
        }
        HavingExprAst::CompareAgg {
            left_agg,
            left_col,
            op,
            right_agg,
            right_col,
        } => {
            let left_col_const = extract_field(ctx, left_col)?;
            let right_col_const = extract_field(ctx, right_col)?;
            let left_agg_kind = agg_kind_ident(left_agg);
            let right_agg_kind = agg_kind_ident(right_agg);
            let op_ident = op_to_ident(op);
            Ok(quote! {
                rust_ef::query::HavingExpr::CompareAgg {
                    left_agg: rust_ef::query::AggKind::#left_agg_kind,
                    left_col: #left_col_const.to_string(),
                    op: rust_ef::query::CompareOp::#op_ident,
                    right_agg: rust_ef::query::AggKind::#right_agg_kind,
                    right_col: #right_col_const.to_string(),
                }
            })
        }
    }
}

/// Maps an aggregate name (e.g. `"COUNT"`) to the `AggKind` variant ident
/// (`Count`). Aggregate names are validated during parsing.
fn agg_kind_ident(agg: &str) -> Ident {
    let variant = match agg.to_uppercase().as_str() {
        "COUNT" => "Count",
        "SUM" => "Sum",
        "AVG" => "Avg",
        "MIN" => "Min",
        "MAX" => "Max",
        other => unreachable!("invalid aggregate name at codegen: {}", other),
    };
    format_ident!("{}", variant)
}

/// Maps an SQL operator symbol (e.g. `">"`) to the `CompareOp` variant ident
/// (`Gt`). Operators are validated during parsing.
fn op_to_ident(op: &str) -> Ident {
    let variant = match op {
        "=" => "Eq",
        "!=" => "Ne",
        ">" => "Gt",
        ">=" => "Ge",
        "<" => "Lt",
        "<=" => "Le",
        other => unreachable!("invalid operator at codegen: {}", other),
    };
    format_ident!("{}", variant)
}
