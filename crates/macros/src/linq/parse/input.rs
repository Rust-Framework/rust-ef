//! `syn::Parse` implementation for `LinqInput` — the top-level `linq!` macro
//! input grammar.
//!
//! - **predicate**: `|b: T| body` — a reusable condition, expands to
//!   `Predicate<T>`
//! - **query**: `source[, |b: T| body][; clause; clause; ...]` — expands to a
//!   `QueryBuilder<T>` chain
//! - **index / key**: `index |b: T| fields` / `key |b: T| fields` — model
//!   configuration constants

use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{parse::Parse, Expr, ExprUnary, Ident, Token, Type, UnOp};

use super::super::ast::*;

// ---------------------------------------------------------------------------
// LinqInput — top-level dispatch
// ---------------------------------------------------------------------------

impl Parse for LinqInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // `index` / `key` are the only keywords; everything else is a predicate
        // (leading `|`) or a query (source expression).
        if input.peek(Ident) {
            let cursor = input.cursor();
            if let Some((ident, _)) = cursor.ident() {
                match ident.to_string().as_str() {
                    "index" => {
                        let value = parse_value_index_or_key(input, ValueKind::Index)?;
                        return Ok(LinqInput::Value(value));
                    }
                    "key" => {
                        let value = parse_value_index_or_key(input, ValueKind::Key)?;
                        return Ok(LinqInput::Value(value));
                    }
                    _ => {}
                }
            }
        }

        // Predicate form or query form.
        let query = parse_query(input)?;
        Ok(LinqInput::Query(query))
    }
}

enum ValueKind {
    Index,
    Key,
}

/// Parses `index |b: T| <field_or_tuple>` or `key |b: T| <field_or_tuple>`.
fn parse_value_index_or_key(
    input: syn::parse::ParseStream,
    kind: ValueKind,
) -> syn::Result<ValueInput> {
    let _keyword: Ident = input.parse()?;

    let _open: Token![|] = input.parse()?;
    let _param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    let _close: Token![|] = input.parse()?;

    let fields = parse_field_or_tuple(input)?;

    // The param is consumed but unused for field extraction context —
    // we resolve fields against `entity` directly.
    match kind {
        ValueKind::Index => Ok(ValueInput::Index { entity, fields }),
        ValueKind::Key => Ok(ValueInput::Key { entity, fields }),
    }
}

/// Parses a single field `b.col` or a tuple `(b.col1, b.col2, ...)`.
pub(super) fn parse_field_or_tuple(input: syn::parse::ParseStream) -> syn::Result<Vec<Expr>> {
    if input.peek(syn::token::Paren) {
        let content;
        syn::parenthesized!(content in input);
        let mut fields = Vec::new();
        while !content.is_empty() {
            let expr: Expr = content.parse()?;
            fields.push(expr);
            if !content.is_empty() {
                let _comma: Token![,] = content.parse()?;
            }
        }
        Ok(fields)
    } else {
        let expr: Expr = input.parse()?;
        Ok(vec![expr])
    }
}

/// Parses predicate-form or query-form input.
fn parse_query(input: syn::parse::ParseStream) -> syn::Result<QueryInput> {
    // Predicate form: `|b: T| body` (no source).
    if input.peek(Token![|]) {
        let (entity, where_param, where_body) = parse_typed_closure(input)?;
        let order = parse_optional_order(input)?;
        let clauses = parse_optional_clauses(input)?;
        return Ok(QueryInput {
            source: None,
            entity,
            where_param: Some(where_param),
            where_body: Some(where_body),
            order,
            clauses,
        });
    }

    // Query form: the source must be an expression carrying a turbofish entity
    // type (`ctx.set::<Blog>()`, `ctx.set::<Blog>().query()`).
    let source: Expr = input.parse()?;
    if !is_value_source(&source) {
        return Err(syn::Error::new_spanned(
            &source,
            "expected a standalone condition (`linq!(|b: Blog| ...)`) or a query source \
             (`linq!(ctx.set::<Blog>(); ...)`); a bare type name or variable is not a source",
        ));
    }
    let source_entity = source_entity_type(&source).map_err(|_| {
        syn::Error::new_spanned(
            &source,
            "expected a standalone condition (`linq!(|b: Blog| ...)`) or a query source with a \
             turbofish entity type (`linq!(ctx.set::<Blog>(); ...)`)",
        )
    })?;

    // `source; clauses` — pure clause query (no condition).
    if input.peek(Token![;]) {
        let clauses = parse_optional_clauses(input)?;
        return Ok(QueryInput {
            source: Some(source),
            entity: source_entity,
            where_param: None,
            where_body: None,
            order: None,
            clauses,
        });
    }

    // `source, |b: T| body` or `source, |b| body` (entity inferred from source).
    let _comma: Token![,] = input.parse().map_err(|_| {
        syn::Error::new_spanned(
            &source,
            "expected `;` (clauses) or `, |b: T| ...` (condition) after the query source",
        )
    })?;
    let (entity, where_param, where_body) = parse_closure_with_inference(input, &source_entity)?;
    let order = parse_optional_order(input)?;
    let clauses = parse_optional_clauses(input)?;
    Ok(QueryInput {
        source: Some(source),
        entity,
        where_param: Some(where_param),
        where_body: Some(where_body),
        order,
        clauses,
    })
}

/// A bare single-segment path is a type name or a variable, not a value the
/// macro can turn into a `QueryBuilder`; it needs a turbofish source instead.
fn is_value_source(expr: &Expr) -> bool {
    !matches!(expr, Expr::Path(p) if p.path.segments.len() == 1)
}

/// Tries to extract the entity type from a source expression.
/// Handles `ctx.set::<Blog>()` and `ctx.set::<Blog>().query()`.
fn source_entity_type(expr: &Expr) -> syn::Result<Type> {
    // Walk method-call chains to find a turbofish `::<Type>` argument.
    let mut current = expr;
    loop {
        match current {
            Expr::MethodCall(call) => {
                if let Some(ty) = call
                    .turbofish
                    .as_ref()
                    .and_then(|tf| tf.args.first())
                    .and_then(|arg| match arg {
                        syn::GenericArgument::Type(ty) => Some(ty.clone()),
                        _ => None,
                    })
                {
                    return Ok(ty);
                }
                current = &call.receiver;
            }
            Expr::Call(call) => {
                current = &call.func;
            }
            _ => break,
        }
    }
    // Fallback: treat as entity type path (e.g. `Blog`).
    expr_as_entity_type(expr)
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

/// Parses `|param: Type| body` — returns (entity_type, param, body).
fn parse_typed_closure(input: syn::parse::ParseStream) -> syn::Result<(Type, Ident, Expr)> {
    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    let _close: Token![|] = input.parse()?;
    let body = parse_expr_until_fat_arrow_or_semi(input)?;
    Ok((entity, param, body))
}

/// Parses a closure that may be typed (`|b: T| body`) or untyped
/// (`|b| body`). When untyped, the entity type is taken from `fallback`
/// (extracted from the source expression's turbofish).
fn parse_closure_with_inference(
    input: syn::parse::ParseStream,
    fallback: &Type,
) -> syn::Result<(Type, Ident, Expr)> {
    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;

    if input.peek(Token![:]) {
        // Typed closure: |param: Type| body
        let _colon: Token![:] = input.parse()?;
        let entity: Type = input.parse()?;
        let _close: Token![|] = input.parse()?;
        let body = parse_expr_until_fat_arrow_or_semi(input)?;
        Ok((entity, param, body))
    } else {
        // Untyped closure: |param| body — entity type inferred from source
        let _close: Token![|] = input.parse()?;
        let body = parse_expr_until_fat_arrow_or_semi(input)?;
        Ok((fallback.clone(), param, body))
    }
}

/// Parses optional `=> field` or `=> -field` order clause.
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

/// Parses optional `; clause; clause; ...` clause list.
fn parse_optional_clauses(input: syn::parse::ParseStream) -> syn::Result<Vec<LinqClause>> {
    let mut clauses = Vec::new();
    while input.peek(Token![;]) {
        let _semi: Token![;] = input.parse()?;
        if input.is_empty() {
            break; // trailing semicolon
        }
        // Collect tokens until next `;` or EOF, then parse as a clause.
        let clause_tokens = collect_until_semi(input)?;
        if clause_tokens.is_empty() {
            break;
        }
        let clause: LinqClause = syn::parse2(clause_tokens)?;
        clauses.push(clause);
    }
    Ok(clauses)
}

fn collect_until_semi(input: syn::parse::ParseStream) -> syn::Result<TokenStream2> {
    let mut tokens = TokenStream2::new();
    while !input.is_empty() && !input.peek(Token![;]) {
        let tt: TokenTree = input.parse()?;
        tokens.extend(std::iter::once(tt));
    }
    Ok(tokens)
}

pub(super) fn parse_expr_until_fat_arrow_or_semi(
    input: syn::parse::ParseStream,
) -> syn::Result<Expr> {
    let mut tokens = TokenStream2::new();
    while !input.is_empty() {
        if input.peek(Token![=>]) || input.peek(Token![;]) {
            break;
        }
        let tt: TokenTree = input.parse()?;
        tokens.extend(std::iter::once(tt));
    }
    if tokens.is_empty() {
        // Empty body — treat as `true` (no filter, pure clause query).
        return Ok(syn::parse_quote!(true));
    }
    syn::parse2(tokens)
}
