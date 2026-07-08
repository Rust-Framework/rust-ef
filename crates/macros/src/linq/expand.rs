//! `linq!` macro expansion — the entry point and clause codegen.
//!
//! `expand_linq` is the `#[proc_macro]` entry point called by `lib.rs`.
//! It dispatches to `expand_query` (Form A/B) or `expand_value` (Form C).
//!
//! `expand_clauses` translates each `;`-separated Form B clause into a
//! `QueryBuilder` method-chain fragment. Terminal clauses (`sum`, `count`,
//! `execute_update`, etc.) produce the final awaitable expression.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Expr, ExprField, ExprLit, ExprPath, Ident, Lit, Member, Type};

use super::ast::*;
use super::compile::{compile_bool_expr, compile_expr, compile_having_expr, compile_order};
use super::context::{extract_field, extract_field_nav, extract_value, LinqCtx};

// ---------------------------------------------------------------------------
// expand_linq — entry point
// ---------------------------------------------------------------------------

pub fn expand_linq(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as LinqInput);
    let result = match input {
        LinqInput::Query(q) => expand_query(&q),
        LinqInput::Value(v) => expand_value(&v),
    };
    match result {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// ---------------------------------------------------------------------------
// Form A / B expansion
// ---------------------------------------------------------------------------

fn expand_query(input: &QueryInput) -> syn::Result<TokenStream2> {
    let entity = &input.entity;
    let where_param = input.where_param.as_ref();

    // Compile the where body into a method-chain fragment.
    let where_chain = if let Some(body) = &input.where_body {
        // `true` body means no actual filter — emit nothing.
        if is_true_expr(body) {
            quote! {}
        } else {
            let ctx = LinqCtx::single(entity, where_param);
            compile_expr(&ctx, body)?
        }
    } else {
        quote! {}
    };

    // Form A `=> order` syntax.
    let order_chain = if let Some(order) = &input.order {
        let ctx = LinqCtx::single(entity, where_param);
        compile_order(&ctx, &order.body, order.descending)?
    } else {
        quote! {}
    };

    // If no source and no clauses → Form A reusable closure.
    if input.source.is_none() && input.clauses.is_empty() {
        return Ok(quote! {
            |__qb: rust_ef::query::QueryBuilder<#entity>| {
                __qb #where_chain #order_chain
            }
        });
    }

    // Build the base builder expression (source + optional filter).
    // Always wrap in .filter() to normalize DbSet → QueryBuilder, even when
    // the where chain is empty (identity closure).
    let base = match &input.source {
        Some(source) => {
            quote! {
                (#source).filter(|__qb: rust_ef::query::QueryBuilder<#entity>| {
                    __qb #where_chain #order_chain
                })
            }
        }
        None => {
            // No source but has clauses — this is an error for Form B.
            // But Form A with order but no source is valid (reusable closure with order).
            if input.clauses.is_empty() {
                return Ok(quote! {
                    |__qb: rust_ef::query::QueryBuilder<#entity>| {
                        __qb #where_chain #order_chain
                    }
                });
            }
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "Form B (multi-clause) requires a source expression, e.g. `linq!(ctx.set::<Blog>(); ...)`",
            ));
        }
    };

    // Expand clauses.
    let clause_chain = expand_clauses(input, entity)?;

    Ok(quote! { #base #clause_chain })
}

/// Returns true if the expression is the literal `true`.
fn is_true_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(ExprLit { lit: Lit::Bool(b), .. }) if b.value)
}

// ---------------------------------------------------------------------------
// Clause expansion
// ---------------------------------------------------------------------------

fn expand_clauses(input: &QueryInput, entity: &Type) -> syn::Result<TokenStream2> {
    let mut chain = quote! {};
    let mut set_clauses: Vec<TokenStream2> = Vec::new();
    let mut terminal: Option<TokenStream2> = None;
    let mut in_update_mode = false;

    let ctx = LinqCtx::single(entity, input.where_param.as_ref());

    for clause in &input.clauses {
        if terminal.is_some() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "no clauses may follow a terminal clause (sum/avg/min/max/count/execute_update)",
            ));
        }

        match clause {
            LinqClause::Include { primary, nested } => {
                let primary_const = extract_field_nav(&ctx, primary)?;
                chain = quote! { #chain .include_internal(#primary_const) };
                for n in nested {
                    // Nested `then` fields can't be type-resolved at compile time
                    // (entity type transition is runtime knowledge). Emit the
                    // field name as a string literal; `then_include_internal`
                    // resolves it via entity metadata at query time.
                    let field_name = extract_field_name_only(n)?;
                    chain = quote! { #chain .then_include_internal(#field_name) };
                }
            }
            LinqClause::OrderBy { field, descending } => {
                let col = extract_field(&ctx, field)?;
                if *descending {
                    chain = quote! { #chain .order_by_desc_column(#col) };
                } else {
                    chain = quote! { #chain .order_by_column(#col) };
                }
            }
            LinqClause::GroupBy { fields } => {
                let cols = extract_field_array(&ctx, fields)?;
                chain = quote! { #chain .group_by_internal(#cols) };
            }
            LinqClause::Select { fields } => {
                let cols = extract_field_array(&ctx, fields)?;
                chain = quote! { #chain .select_internal(#cols) };
            }
            LinqClause::HavingExpr { expr } => {
                let having_code = compile_having_expr(expr, &ctx)?;
                chain = quote! {
                    #chain .having_expr_internal(#having_code)
                };
            }
            LinqClause::Distinct => {
                chain = quote! { #chain .distinct() };
            }
            LinqClause::Take(n) => {
                chain = quote! { #chain .take(#n) };
            }
            LinqClause::Skip(n) => {
                chain = quote! { #chain .skip(#n) };
            }
            LinqClause::Window {
                func,
                column,
                partition_by,
                order_by,
                alias,
            } => {
                let func_str = func.as_str();
                // Column: Option<&'static str>. Ranking functions have no column.
                let col_tokens: TokenStream2 = match column {
                    Some(expr) => {
                        let col = extract_field(&ctx, expr)?;
                        quote! { Some(#col) }
                    }
                    None => quote! { None },
                };
                // partition_by: &'static [&'static str]
                let pb_tokens: Vec<TokenStream2> = partition_by
                    .iter()
                    .map(|e| extract_field(&ctx, e))
                    .collect::<syn::Result<_>>()?;
                let pb_arr = quote! { &[#(#pb_tokens),*] };
                // order_by: &'static [(&'static str, bool)]
                let ob_tokens: Vec<TokenStream2> = order_by
                    .iter()
                    .map(|(e, d)| {
                        let col = extract_field(&ctx, e)?;
                        Ok::<_, syn::Error>(quote! { (#col, #d) })
                    })
                    .collect::<syn::Result<_>>()?;
                let ob_arr = quote! { &[#(#ob_tokens),*] };
                let alias_str = alias.as_str();
                chain = quote! {
                    #chain .window_internal(
                        #func_str,
                        #col_tokens,
                        #pb_arr,
                        #ob_arr,
                        #alias_str,
                    )
                };
            }
            LinqClause::With {
                name,
                entity,
                param,
                body,
                recursive,
                link,
            } => {
                // Create a separate LinqCtx for the CTE's entity type so that
                // field references in the closure body (e.g. `e.salary`)
                // resolve to the CTE entity's column constants, not the main
                // query's entity.
                let cte_ctx = LinqCtx::single(entity, Some(param));
                let bool_expr_code = compile_bool_expr(&cte_ctx, body)?;
                let name_str = name.as_str();
                if *recursive {
                    let (fk_expr, pk_expr) = link
                        .as_ref()
                        .expect("recursive CTE must have `link <fk> to <pk>`");
                    let fk_col = extract_field_name_only(fk_expr)?;
                    let pk_col = extract_field_name_only(pk_expr)?;
                    chain = quote! {
                        #chain .with_recursive_cte_typed(
                            #name_str,
                            <#entity>::TABLE,
                            #fk_col,
                            #pk_col,
                            #bool_expr_code,
                        )
                    };
                } else {
                    // `<Entity>::TABLE` is a `&'static str` constant emitted by
                    // `#[derive(EntityType)]`.
                    chain = quote! {
                        #chain .with_cte_typed(
                            #name_str,
                            <#entity>::TABLE,
                            #bool_expr_code,
                        )
                    };
                }
            }
            LinqClause::From { name } => {
                let name_str = name.as_str();
                chain = quote! { #chain .from_cte(#name_str) };
            }
            LinqClause::InnerJoin {
                params,
                left,
                right,
            } => {
                let (table, left_col, right_col) = expand_join(params, left, right)?;
                chain = quote! { #chain .inner_join_internal(#table, #left_col, #right_col) };
            }
            LinqClause::LeftJoin {
                params,
                left,
                right,
            } => {
                let (table, left_col, right_col) = expand_join(params, left, right)?;
                chain = quote! { #chain .left_join_internal(#table, #left_col, #right_col) };
            }
            LinqClause::RightJoin {
                params,
                left,
                right,
            } => {
                let (table, left_col, right_col) = expand_join(params, left, right)?;
                chain = quote! { #chain .right_join_internal(#table, #left_col, #right_col) };
            }
            LinqClause::FullJoin {
                params,
                left,
                right,
            } => {
                let (table, left_col, right_col) = expand_join(params, left, right)?;
                chain = quote! { #chain .full_join_internal(#table, #left_col, #right_col) };
            }
            LinqClause::CrossJoin { entity, .. } => {
                let table = quote! { <#entity>::TABLE };
                chain = quote! { #chain .cross_join_internal(#table) };
            }
            LinqClause::Union(expr) => {
                chain = quote! { #chain .union_internal(#expr) };
            }
            LinqClause::UnionAll(expr) => {
                chain = quote! { #chain .union_all_internal(#expr) };
            }
            LinqClause::Intersect(expr) => {
                chain = quote! { #chain .intersect_internal(#expr) };
            }
            LinqClause::Except(expr) => {
                chain = quote! { #chain .except_internal(#expr) };
            }
            LinqClause::Set { field, value } => {
                if !in_update_mode {
                    // `set` before `execute_update` — we'll emit `.execute_update()` first.
                    chain = quote! { #chain .execute_update() };
                    in_update_mode = true;
                }
                let col = extract_field(&ctx, field)?;
                let val = extract_value(value)?;
                set_clauses.push(
                    quote! { .set_column_internal(#col, rust_ef::provider::DbValue::from(#val)) },
                );
            }
            LinqClause::ExecuteUpdate => {
                if !in_update_mode {
                    chain = quote! { #chain .execute_update() };
                    in_update_mode = true;
                }
                let sets = set_clauses.iter().cloned().collect::<TokenStream2>();
                terminal = Some(quote! { #sets .execute() });
            }
            LinqClause::Sum(field) => {
                let col = extract_field(&ctx, field)?;
                terminal = Some(quote! { .sum_internal(#col) });
            }
            LinqClause::Avg(field) => {
                let col = extract_field(&ctx, field)?;
                terminal = Some(quote! { .avg_internal(#col) });
            }
            LinqClause::Min(field) => {
                let col = extract_field(&ctx, field)?;
                terminal = Some(quote! { .min_internal(#col) });
            }
            LinqClause::Max(field) => {
                let col = extract_field(&ctx, field)?;
                terminal = Some(quote! { .max_internal(#col) });
            }
            LinqClause::Count => {
                terminal = Some(quote! { .count() });
            }
        }
    }

    if let Some(term) = terminal {
        Ok(quote! { #chain #term })
    } else {
        Ok(chain)
    }
}

/// Expands a join clause: returns (table, left_col, right_col) constant references.
fn expand_join(
    params: &[(Ident, Type)],
    left: &Expr,
    right: &Expr,
) -> syn::Result<(TokenStream2, TokenStream2, TokenStream2)> {
    if params.len() != 2 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "join requires exactly two parameters: |a: T1, b: T2| ...",
        ));
    }
    let (left_param, left_entity) = &params[0];
    let (right_param, right_entity) = &params[1];

    let left_ctx = LinqCtx::multi(left_entity, left_param, params);
    let right_ctx = LinqCtx::multi(right_entity, right_param, params);

    let left_col = extract_field(&left_ctx, left)?;
    let right_col = extract_field(&right_ctx, right)?;

    // The joined table is the right entity's TABLE constant.
    let table = quote! { <#right_entity>::TABLE };

    Ok((table, left_col, right_col))
}

/// Extracts field names as a `&'static [&'static str]` array.
fn extract_field_array(ctx: &LinqCtx<'_>, fields: &[Expr]) -> syn::Result<TokenStream2> {
    let cols: Vec<TokenStream2> = fields
        .iter()
        .map(|f| extract_field(ctx, f))
        .collect::<syn::Result<_>>()?;
    Ok(quote! { &[#(#cols),*] })
}

/// Extracts just the field name as a `String` (for `then` include nested paths).
/// The caller interpolates this into `quote!` which produces a string literal.
fn extract_field_name_only(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Field(ExprField { member, .. }) => match member {
            Member::Named(name) => Ok(name.to_string()),
            _ => Err(syn::Error::new_spanned(expr, "expected named field")),
        },
        Expr::Path(ExprPath { path, .. }) if path.segments.len() == 1 => {
            Ok(path.segments[0].ident.to_string())
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "expected field access like `b.comments`",
        )),
    }
}

// ---------------------------------------------------------------------------
// Form C expansion (value-producing)
// ---------------------------------------------------------------------------

fn expand_value(input: &ValueInput) -> syn::Result<TokenStream2> {
    match input {
        ValueInput::Filter {
            entity,
            param,
            body,
        } => {
            let ctx = LinqCtx::single(entity, Some(param));
            compile_bool_expr(&ctx, body)
        }
        ValueInput::Index { entity, fields } => expand_field_array(entity, fields),
        ValueInput::Key { entity, fields } => expand_field_array(entity, fields),
    }
}

fn expand_field_array(entity: &Type, fields: &[Expr]) -> syn::Result<TokenStream2> {
    let ctx = LinqCtx::single(entity, None);
    let cols: Vec<TokenStream2> = fields
        .iter()
        .map(|f| extract_field(&ctx, f))
        .collect::<syn::Result<_>>()?;
    Ok(quote! { &[#(#cols),*] })
}
