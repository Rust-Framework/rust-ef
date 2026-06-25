//! `linq!()` ? compile-time LINQ-to-SQL.
//!
//! ```ignore
//! // Query directly from a DbSet / QueryBuilder
//! linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;
//!
//! // Reusable expression tree
//! let expr = linq!(|b: Blog| b.rating > min_rating);
//! ctx.set::<Blog>().filter(expr).to_list().await?;
//! ```

use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{
    parse::Parse, parse_macro_input, BinOp, Expr, ExprField, ExprLit, ExprMethodCall, ExprPath,
    ExprUnary, Ident, Lit, Member, Token, Type, UnOp,
};

struct LinqInput {
    source: Option<Expr>,
    entity: Type,
    where_clause: LinqWhere,
    order: Option<LinqOrder>,
}

struct LinqWhere {
    param: Option<Ident>,
    body: Expr,
}

struct LinqOrder {
    body: Expr,
    descending: bool,
}

impl Parse for LinqInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        if input.peek(Token![|]) {
            let (entity, where_clause) = parse_typed_closure_where(input)?;
            let order = parse_optional_order(input)?;
            return Ok(LinqInput {
                source: None,
                entity,
                where_clause,
                order,
            });
        }

        let first: Expr = input.parse()?;
        let _comma: Token![,] = input.parse()?;

        if input.peek(Token![|]) {
            if is_source_expr(&first) {
                let (entity, where_clause) = parse_typed_closure_where(input)?;
                let order = parse_optional_order(input)?;
                return Ok(LinqInput {
                    source: Some(first),
                    entity,
                    where_clause,
                    order,
                });
            }
            let entity = expr_as_entity_type(&first)?;
            let where_clause = parse_untyped_closure_where(input)?;
            let order = parse_optional_order(input)?;
            return Ok(LinqInput {
                source: None,
                entity,
                where_clause,
                order,
            });
        }

        let where_clause = parse_where_rest(input)?;
        let entity = expr_as_entity_type(&first)?;
        let order = parse_optional_order(input)?;
        Ok(LinqInput {
            source: None,
            entity,
            where_clause,
            order,
        })
    }
}

fn is_source_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Path(ExprPath { path, .. }) => path.segments.len() != 1,
        _ => true,
    }
}

fn expr_as_entity_type(expr: &Expr) -> syn::Result<Type> {
    match expr {
        Expr::Path(path) => syn::parse2(quote! { #path }),
        _ => Err(syn::Error::new_spanned(
            expr,
            "expected entity type, e.g. `Blog`",
        )),
    }
}

fn parse_typed_closure_where(input: syn::parse::ParseStream) -> syn::Result<(Type, LinqWhere)> {
    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    let _close: Token![|] = input.parse()?;
    let body = parse_expr_until_fat_arrow(input)?;
    Ok((
        entity,
        LinqWhere {
            param: Some(param),
            body,
        },
    ))
}

fn parse_untyped_closure_where(input: syn::parse::ParseStream) -> syn::Result<LinqWhere> {
    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _close: Token![|] = input.parse()?;
    let body = parse_expr_until_fat_arrow(input)?;
    Ok(LinqWhere {
        param: Some(param),
        body,
    })
}

fn parse_where_rest(input: syn::parse::ParseStream) -> syn::Result<LinqWhere> {
    let body = parse_expr_until_fat_arrow(input)?;
    Ok(LinqWhere {
        param: None,
        body,
    })
}

fn parse_optional_order(input: syn::parse::ParseStream) -> syn::Result<Option<LinqOrder>> {
    if !input.peek(Token![=>]) {
        return Ok(None);
    }
    let _arrow: Token![=>] = input.parse()?;
    let order_body: Expr = input.parse()?;
    let (field_expr, descending) = parse_order_expr(&order_body)?;
    Ok(Some(LinqOrder {
        body: field_expr,
        descending,
    }))
}

fn parse_expr_until_fat_arrow(input: syn::parse::ParseStream) -> syn::Result<Expr> {
    let mut tokens = TokenStream2::new();
    while !input.is_empty() {
        if input.peek(Token![=>]) {
            break;
        }
        let tt: TokenTree = input.parse()?;
        tokens.extend(std::iter::once(tt));
    }
    syn::parse2(tokens)
}

fn parse_order_expr(expr: &Expr) -> syn::Result<(Expr, bool)> {
    if let Expr::Unary(ExprUnary {
        op: UnOp::Neg(_),
        expr: inner,
        ..
    }) = expr
    {
        return Ok((*inner.clone(), true));
    }
    Ok((expr.clone(), false))
}

pub fn expand_linq(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as LinqInput);
    let ctx = LinqCtx {
        entity: &input.entity,
        param: input.where_clause.param.as_ref(),
    };

    let where_chain = match compile_expr(&ctx, &input.where_clause.body) {
        Ok(c) => c,
        Err(e) => return e.to_compile_error().into(),
    };

    let order_chain = if let Some(order) = &input.order {
        let order_ctx = LinqCtx {
            entity: &input.entity,
            param: input.where_clause.param.as_ref(),
        };
        match compile_order(&order_ctx, &order.body, order.descending) {
            Ok(c) => c,
            Err(e) => return e.to_compile_error().into(),
        }
    } else {
        quote! {}
    };

    let entity = &input.entity;
    let apply = quote! {
        |__qb: rust_ef::query::QueryBuilder<#entity>| {
            __qb #where_chain #order_chain
        }
    };

    TokenStream::from(match input.source {
        Some(source) => quote! {{
            (#source).filter(#apply)
        }},
        None => quote! { #apply },
    })
}

struct LinqCtx<'a> {
    entity: &'a Type,
    param: Option<&'a Ident>,
}

fn compile_expr(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
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

fn compile_not(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
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
        Expr::MethodCall(call) if call.method == "contains" => compile_contains(ctx, call, true),
        _ => compile_negated_comparison(ctx, expr),
    }
}

fn compile_bool_member(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    let column = extract_field(ctx, expr)?;
    Ok(quote! {
        .filter_column(#column, "=", rust_ef::provider::DbValue::Bool(true))
    })
}

fn compile_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
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

fn compile_negated_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Group(g) => compile_negated_comparison(ctx, &g.expr),
        Expr::Paren(p) => compile_negated_comparison(ctx, &p.expr),
        Expr::MethodCall(call) if call.method == "contains" => {
            compile_contains(ctx, call, true)
        }
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

/// `b.url.contains("x")` ? LIKE; `ids.contains(b.id)` ? IN (LINQ style).
fn compile_contains(
    ctx: &LinqCtx<'_>,
    call: &ExprMethodCall,
    negate: bool,
) -> syn::Result<TokenStream2> {
    let arg = call.args.first().ok_or_else(|| {
        syn::Error::new_spanned(call, "`.contains()` requires one argument")
    })?;

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

fn compile_method(ctx: &LinqCtx<'_>, call: &ExprMethodCall) -> syn::Result<TokenStream2> {
    let method = call.method.to_string();

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

fn compile_order(ctx: &LinqCtx<'_>, expr: &Expr, descending: bool) -> syn::Result<TokenStream2> {
    let column = extract_field(ctx, expr)?;
    if descending {
        Ok(quote! { .order_by_desc_column(#column) })
    } else {
        Ok(quote! { .order_by_column(#column) })
    }
}

fn extract_field(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Path(ExprPath { path, .. }) if path.segments.len() == 1 => {
            let field = path.segments[0].ident.to_string();
            Ok(field_column_const(ctx.entity, &field))
        }
        Expr::Field(ExprField { base, member, .. }) => {
            if let Member::Named(field_name) = member {
                if let Some(param) = ctx.param {
                    if let Expr::Path(ExprPath { path, .. }) = &**base {
                        if path.segments.len() == 1 && path.segments[0].ident == *param {
                            return Ok(field_column_const(
                                ctx.entity,
                                &field_name.to_string(),
                            ));
                        }
                    }
                }
                if let Expr::Path(ExprPath { path, .. }) = &**base {
                    if type_path_matches(path, ctx.entity) {
                        return Ok(field_column_const(
                            ctx.entity,
                            &field_name.to_string(),
                        ));
                    }
                }
                Ok(field_column_const(ctx.entity, &field_name.to_string()))
            } else {
                Err(syn::Error::new_spanned(expr, "expected field name"))
            }
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "expected field access like `b.rating`",
        )),
    }
}

fn type_path_matches(path: &syn::Path, entity: &Type) -> bool {
    let entity_name = if let Type::Path(p) = entity {
        p.path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    path.segments
        .last()
        .is_some_and(|s| s.ident == entity_name)
}

fn field_column_const(entity: &Type, field: &str) -> TokenStream2 {
    let const_name = Ident::new(
        &format!("COLUMN_{}", field.to_uppercase()),
        proc_macro2::Span::call_site(),
    );
    quote! { #entity::#const_name }
}

fn extract_value(expr: &Expr) -> syn::Result<TokenStream2> {
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => match &lit {
            Lit::Int(i) => Ok(quote! { #i }),
            Lit::Float(f) => Ok(quote! { #f }),
            Lit::Str(s) => Ok(quote! { #s }),
            Lit::Bool(b) => Ok(quote! { #b }),
            _ => Err(syn::Error::new_spanned(lit, "unsupported literal type")),
        },
        Expr::Path(p) if p.path.is_ident("None") => Ok(quote! { None::<String> }),
        other => Ok(quote! { #other }),
    }
}
