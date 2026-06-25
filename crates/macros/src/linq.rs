//! `linq!()` — compile-time LINQ-to-SQL.
//!
//! The single DSL entry point for all database operations. Three syntactic
//! forms are supported:
//!
//! **Form A** (filter closure, existing):
//! ```ignore
//! // Reusable filter closure
//! let expr = linq!(|b: Blog| b.rating > 0.5);
//! set.filter(expr).to_list().await?;
//!
//! // Direct query from a source
//! linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;
//! ```
//!
//! **Form B** (multi-clause query, new):
//! ```ignore
//! // Full query in one expression
//! linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0.5;
//!     include b.posts then b.comments;
//!     order_by b.created_at desc;
//! ).to_list().await?;
//!
//! // Aggregate terminal
//! let total: f64 = linq!(ctx.set::<Blog>(); sum b.views).await?;
//! ```
//!
//! **Form C** (value-producing, for ModelBuilder configuration):
//! ```ignore
//! builder.has_query_filter(linq!(filter |b: Blog| b.deleted_at.is_null()));
//! builder.has_index(linq!(index |b: Blog| (b.author_id, b.created_at)));
//! builder.has_key(linq!(key |b: Blog| b.id));
//! ```

use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{
    parse::Parse, parse_macro_input, BinOp, Expr, ExprCall, ExprField, ExprLit, ExprMethodCall,
    ExprPath, ExprUnary, Ident, Lit, Member, Token, Type, UnOp,
};

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

/// Top-level dispatch: query form (A/B) vs. value form (C).
enum LinqInput {
    Query(QueryInput),
    Value(ValueInput),
}

/// Form A (filter closure) and Form B (multi-clause query) share this shape.
struct QueryInput {
    /// The source `QueryBuilder<T>` expression (e.g. `ctx.set::<Blog>()`).
    /// `None` for Form A reusable closures.
    source: Option<Expr>,
    entity: Type,
    where_param: Option<Ident>,
    /// `None` for pure-clause queries (Form B without a where closure).
    where_body: Option<Expr>,
    /// Form A `=> field` order syntax (kept for backward compatibility).
    order: Option<LinqOrder>,
    /// Form B `;`-separated clauses.
    clauses: Vec<LinqClause>,
}

struct LinqOrder {
    body: Expr,
    descending: bool,
}

/// Form C value-producing inputs.
enum ValueInput {
    /// `linq!(filter |b: T| <bool_expr>)` → `BoolExpr`
    Filter { entity: Type, param: Ident, body: Expr },
    /// `linq!(index |b: T| (f1, f2, ...))` → `&'static [&'static str]`
    Index { entity: Type, fields: Vec<Expr> },
    /// `linq!(key |b: T| (f1, f2, ...))` → `&'static [&'static str]`
    Key { entity: Type, fields: Vec<Expr> },
}

/// A single `;`-separated clause in Form B.
enum LinqClause {
    /// `include b.posts then b.comments then b.author`
    Include { primary: Expr, nested: Vec<Expr> },
    /// `order_by b.created_at [asc|desc]`
    OrderBy { field: Expr, descending: bool },
    /// `group_by (b.cat, b.author)` or `group_by b.cat`
    GroupBy { fields: Vec<Expr> },
    /// `select (b.id, b.title)` or `select b.id`
    Select { fields: Vec<Expr> },
    /// `having count(b.id) > 1`
    Having { agg: String, col: Expr, op: String, value: Expr },
    /// `sum b.views` (terminal)
    Sum(Expr),
    /// `avg b.rating` (terminal)
    Avg(Expr),
    /// `min b.rating` (terminal)
    Min(Expr),
    /// `max b.rating` (terminal)
    Max(Expr),
    /// `count` (terminal)
    Count,
    /// `distinct`
    Distinct,
    /// `set b.views, 10` (only valid before `execute_update`)
    Set { field: Expr, value: Expr },
    /// `inner_join |a: T1, b: T2| a.col == b.col`
    InnerJoin { params: Vec<(Ident, Type)>, left: Expr, right: Expr },
    /// `left_join |a: T1, b: T2| a.col == b.col`
    LeftJoin { params: Vec<(Ident, Type)>, left: Expr, right: Expr },
    /// `execute_update` (terminal, triggers bulk update)
    ExecuteUpdate,
    /// `take N`
    Take(Expr),
    /// `skip N`
    Skip(Expr),
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

impl Parse for LinqInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        // Form C dispatch: first token is `filter` / `index` / `key` keyword.
        if input.peek(Ident) {
            let cursor = input.cursor();
            if let Some((ident, _)) = cursor.ident() {
                match ident.to_string().as_str() {
                    "filter" => {
                        let value = parse_value_filter(input)?;
                        return Ok(LinqInput::Value(value));
                    }
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

        // Form A / B: parse as query.
        let query = parse_query(input)?;
        Ok(LinqInput::Query(query))
    }
}

enum ValueKind {
    Index,
    Key,
}

/// Parses `filter |b: T| <body>`.
fn parse_value_filter(input: syn::parse::ParseStream) -> syn::Result<ValueInput> {
    let keyword: Ident = input.parse()?;
    debug_assert_eq!(keyword, "filter");

    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    let _close: Token![|] = input.parse()?;

    let body: Expr = input.parse()?;

    Ok(ValueInput::Filter { entity, param, body })
}

/// Parses `index |b: T| <field_or_tuple>` or `key |b: T| <field_or_tuple>`.
fn parse_value_index_or_key(
    input: syn::parse::ParseStream,
    kind: ValueKind,
) -> syn::Result<ValueInput> {
    let keyword: Ident = input.parse()?;
    let kw_str = keyword.to_string();

    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    let _close: Token![|] = input.parse()?;

    let fields = parse_field_or_tuple(input)?;

    // The param is consumed but unused for field extraction context —
    // we resolve fields against `entity` directly.
    let _ = param;

    match kind {
        ValueKind::Index => Ok(ValueInput::Index { entity, fields }),
        ValueKind::Key => Ok(ValueInput::Key { entity, fields }),
    }
    .map(|v| {
        // Suppress unused warning
        let _ = kw_str;
        v
    })
}

/// Parses a single field `b.col` or a tuple `(b.col1, b.col2, ...)`.
fn parse_field_or_tuple(input: syn::parse::ParseStream) -> syn::Result<Vec<Expr>> {
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

/// Parses Form A / Form B query input.
fn parse_query(input: syn::parse::ParseStream) -> syn::Result<QueryInput> {
    // Form A without source: `|b: T| body [=> order] [; clauses]`
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

    // Source expression (or entity type for untyped closure form)
    let first: Expr = input.parse()?;

    // `source, |b: T| body` or `source; clauses` or `source, |b: T| body; clauses`
    if input.peek(Token![,]) {
        let _comma: Token![,] = input.parse()?;

        if input.peek(Token![|]) {
            // Typed closure: source is a QueryBuilder
            if is_source_expr(&first) {
                let (entity, where_param, where_body) = parse_typed_closure(input)?;
                let order = parse_optional_order(input)?;
                let clauses = parse_optional_clauses(input)?;
                return Ok(QueryInput {
                    source: Some(first),
                    entity,
                    where_param: Some(where_param),
                    where_body: Some(where_body),
                    order,
                    clauses,
                });
            }
            // Untyped closure: `Blog, |b| body` — first is entity type
            let entity = expr_as_entity_type(&first)?;
            let (where_param, where_body) = parse_untyped_closure(input)?;
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

        // `source, <expr>` without closure — treat as entity type + where body
        // (legacy: `linq!(Blog, b.rating > 5)`)
        let entity = expr_as_entity_type(&first)?;
        let where_body: Expr = input.parse()?;
        let order = parse_optional_order(input)?;
        let clauses = parse_optional_clauses(input)?;
        return Ok(QueryInput {
            source: None,
            entity,
            where_param: None,
            where_body: Some(where_body),
            order,
            clauses,
        });
    }

    // `source; clauses` — pure clause query (no where closure)
    if input.peek(Token![;]) {
        let entity = source_entity_type(&first)?;
        let clauses = parse_optional_clauses(input)?;
        return Ok(QueryInput {
            source: Some(first),
            entity,
            where_param: None,
            where_body: None,
            order: None,
            clauses,
        });
    }

    // Legacy: `Blog => order` or `Blog, body` already handled above.
    // Try: entity type + where body (no comma, e.g. `linq!(Blog b.rating > 5)`)
    let entity = expr_as_entity_type(&first)?;
    let where_body = parse_where_rest(input)?;
    let order = parse_optional_order(input)?;
    let clauses = parse_optional_clauses(input)?;
    Ok(QueryInput {
        source: None,
        entity,
        where_param: None,
        where_body: Some(where_body),
        order,
        clauses,
    })
}

fn is_source_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Path(ExprPath { path, .. }) => path.segments.len() != 1,
        _ => true,
    }
}

/// Tries to extract the entity type from a source expression.
/// For `ctx.set::<Blog>()`, extracts `Blog`. Falls back to treating as entity type.
fn source_entity_type(expr: &Expr) -> syn::Result<Type> {
    // For source expressions like `ctx.set::<Blog>()`, we can't easily extract
    // the entity type at parse time. We require the user to use typed closures
    // for Form B with a where clause. For pure-clause Form B, we extract from
    // the first clause's field access at expansion time.
    //
    // As a fallback, if the source is a simple path (entity type), use it.
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
fn parse_typed_closure(
    input: syn::parse::ParseStream,
) -> syn::Result<(Type, Ident, Expr)> {
    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    let _close: Token![|] = input.parse()?;
    let body = parse_expr_until_fat_arrow_or_semi(input)?;
    Ok((entity, param, body))
}

/// Parses `|param| body` — returns (param, body). Entity type inferred from context.
fn parse_untyped_closure(
    input: syn::parse::ParseStream,
) -> syn::Result<(Ident, Expr)> {
    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _close: Token![|] = input.parse()?;
    let body = parse_expr_until_fat_arrow_or_semi(input)?;
    Ok((param, body))
}

fn parse_where_rest(input: syn::parse::ParseStream) -> syn::Result<Expr> {
    parse_expr_until_fat_arrow_or_semi(input)
}

/// Parses optional `=> field` or `=> -field` order clause (Form A backward compat).
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

/// Parses optional `; clause; clause; ...` clause list (Form B).
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

fn parse_expr_until_fat_arrow_or_semi(input: syn::parse::ParseStream) -> syn::Result<Expr> {
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

// ---------------------------------------------------------------------------
// LinqClause parsing
// ---------------------------------------------------------------------------

impl Parse for LinqClause {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let keyword: Ident = input.parse()?;
        match keyword.to_string().as_str() {
            "include" => parse_include_rest(input),
            "order_by" => parse_order_by_rest(input),
            "group_by" => parse_group_by_rest(input),
            "select" => parse_select_rest(input),
            "having" => parse_having_rest(input),
            "sum" => {
                let field: Expr = input.parse()?;
                Ok(LinqClause::Sum(field))
            }
            "avg" => {
                let field: Expr = input.parse()?;
                Ok(LinqClause::Avg(field))
            }
            "min" => {
                let field: Expr = input.parse()?;
                Ok(LinqClause::Min(field))
            }
            "max" => {
                let field: Expr = input.parse()?;
                Ok(LinqClause::Max(field))
            }
            "count" => Ok(LinqClause::Count),
            "distinct" => Ok(LinqClause::Distinct),
            "set" => parse_set_rest(input),
            "inner_join" => parse_join_rest(input, false),
            "left_join" => parse_join_rest(input, true),
            "execute_update" => Ok(LinqClause::ExecuteUpdate),
            "take" => {
                let n: Expr = input.parse()?;
                Ok(LinqClause::Take(n))
            }
            "skip" => {
                let n: Expr = input.parse()?;
                Ok(LinqClause::Skip(n))
            }
            other => Err(syn::Error::new(
                keyword.span(),
                format!("unknown linq! clause: `{}`", other),
            )),
        }
    }
}

/// `include b.posts then b.comments then b.author`
fn parse_include_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let primary: Expr = input.parse()?;
    let mut nested = Vec::new();
    while input.peek(Ident) {
        let cursor = input.cursor();
        if let Some((ident, _)) = cursor.ident() {
            if ident == "then" {
                let _then: Ident = input.parse()?;
                let next: Expr = input.parse()?;
                nested.push(next);
            } else {
                break;
            }
        } else {
            break;
        }
    }
    Ok(LinqClause::Include { primary, nested })
}

/// `order_by b.field [asc|desc]`
fn parse_order_by_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let field: Expr = input.parse()?;
    let mut descending = false;
    if input.peek(Ident) {
        let cursor = input.cursor();
        if let Some((ident, _)) = cursor.ident() {
            match ident.to_string().as_str() {
                "asc" => {
                    let _: Ident = input.parse()?;
                }
                "desc" => {
                    let _: Ident = input.parse()?;
                    descending = true;
                }
                _ => {}
            }
        }
    }
    Ok(LinqClause::OrderBy { field, descending })
}

/// `group_by (b.cat, b.author)` or `group_by b.cat`
fn parse_group_by_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let fields = parse_field_or_tuple(input)?;
    Ok(LinqClause::GroupBy { fields })
}

/// `select (b.id, b.title)` or `select b.id`
fn parse_select_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let fields = parse_field_or_tuple(input)?;
    Ok(LinqClause::Select { fields })
}

/// `having count(b.id) > 1`
fn parse_having_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let expr: Expr = input.parse()?;
    let binary = match &expr {
        Expr::Binary(b) => b,
        _ => {
            return Err(syn::Error::new_spanned(
                expr,
                "having expects `agg(field) op value`, e.g. `having count(b.id) > 1`",
            ));
        }
    };

    let (agg, col) = match &*binary.left {
        Expr::Call(ExprCall { func, args, .. }) => {
            let agg = match &**func {
                Expr::Path(p) if p.path.segments.len() == 1 => {
                    p.path.segments[0].ident.to_string()
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        func,
                        "expected aggregate function: count/sum/avg/min/max",
                    ));
                }
            };
            let col = args
                .first()
                .ok_or_else(|| {
                    syn::Error::new_spanned(func, "aggregate function requires a column argument")
                })?
                .clone();
            (agg, col)
        }
        _ => {
            return Err(syn::Error::new_spanned(
                &binary.left,
                "expected aggregate function call, e.g. `count(b.id)`",
            ));
        }
    };

    // Validate aggregate name.
    match agg.to_lowercase().as_str() {
        "count" | "sum" | "avg" | "min" | "max" => {}
        _ => {
            return Err(syn::Error::new_spanned(
                &binary.left,
                "unsupported aggregate; use count/sum/avg/min/max",
            ));
        }
    }

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
                "unsupported comparison operator in having",
            ));
        }
    };
    let value: Expr = (*binary.right).clone();

    Ok(LinqClause::Having {
        agg: agg.to_uppercase(),
        col,
        op: op.to_string(),
        value,
    })
}

/// `set b.col, value`
fn parse_set_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let field: Expr = input.parse()?;
    let _comma: Token![,] = input.parse()?;
    let value: Expr = input.parse()?;
    Ok(LinqClause::Set { field, value })
}

/// `inner_join |a: T1, b: T2| a.col == b.col` or `left_join |...| ...`
fn parse_join_rest(input: syn::parse::ParseStream, is_left: bool) -> syn::Result<LinqClause> {
    let _open: Token![|] = input.parse()?;
    let mut params = Vec::new();
    while !input.peek(Token![|]) {
        let param: Ident = input.parse()?;
        let _colon: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;
        params.push((param, ty));
        if !input.peek(Token![|]) {
            let _comma: Token![,] = input.parse()?;
        }
    }
    let _close: Token![|] = input.parse()?;

    let cond: Expr = input.parse()?;
    let binary = match &cond {
        Expr::Binary(b) if matches!(b.op, BinOp::Eq(_)) => b,
        _ => {
            return Err(syn::Error::new_spanned(
                cond,
                "join condition must be `a.col == b.col`",
            ));
        }
    };

    let left: Expr = (*binary.left).clone();
    let right: Expr = (*binary.right).clone();

    if is_left {
        Ok(LinqClause::LeftJoin { params, left, right })
    } else {
        Ok(LinqClause::InnerJoin { params, left, right })
    }
}

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
    let base = match &input.source {
        Some(source) => {
            if where_chain.is_empty() && order_chain.is_empty() {
                quote! { (#source) }
            } else {
                quote! {
                    (#source).filter(|__qb: rust_ef::query::QueryBuilder<#entity>| {
                        __qb #where_chain #order_chain
                    })
                }
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
            LinqClause::Having { agg, col, op, value } => {
                let col_const = extract_field(&ctx, col)?;
                let val = extract_value(value)?;
                chain = quote! {
                    #chain .having_internal(#agg, #col_const, #op, rust_ef::provider::DbValue::from(#val))
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
            LinqClause::InnerJoin { params, left, right } => {
                let (table, left_col, right_col) = expand_join(params, left, right)?;
                chain = quote! { #chain .inner_join_internal(#table, #left_col, #right_col) };
            }
            LinqClause::LeftJoin { params, left, right } => {
                let (table, left_col, right_col) = expand_join(params, left, right)?;
                chain = quote! { #chain .left_join_internal(#table, #left_col, #right_col) };
            }
            LinqClause::Set { field, value } => {
                if !in_update_mode {
                    // `set` before `execute_update` — we'll emit `.execute_update()` first.
                    chain = quote! { #chain .execute_update() };
                    in_update_mode = true;
                }
                let col = extract_field(&ctx, field)?;
                let val = extract_value(value)?;
                set_clauses.push(quote! { .set_column_internal(#col, rust_ef::provider::DbValue::from(#val)) });
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

/// Extracts just the field name as a string literal (for `then` include nested paths).
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
    .map(|s| quote! { #s }.to_string())
}

// ---------------------------------------------------------------------------
// Form C expansion (value-producing)
// ---------------------------------------------------------------------------

fn expand_value(input: &ValueInput) -> syn::Result<TokenStream2> {
    match input {
        ValueInput::Filter { entity, param, body } => {
            let ctx = LinqCtx::single(entity, Some(param));
            compile_bool_expr(&ctx, body)
        }
        ValueInput::Index { entity, fields } => {
            expand_field_array(entity, fields)
        }
        ValueInput::Key { entity, fields } => {
            expand_field_array(entity, fields)
        }
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

/// Compiles a boolean expression to a `BoolExpr` value (Form C `filter`).
///
/// Mirrors `compile_expr` but emits `BoolExpr::Filter(...)` / `BoolExpr::And(...)`
/// / `BoolExpr::Or(...)` / `BoolExpr::Not(...)` value expressions instead of
/// `QueryBuilder` method chains.
fn compile_bool_expr(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
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

fn compile_bool_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
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
fn compile_bool_method(ctx: &LinqCtx<'_>, call: &ExprMethodCall) -> syn::Result<TokenStream2> {
    let method = call.method.to_string();

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
// LinqCtx — compilation context
// ---------------------------------------------------------------------------

struct LinqCtx<'a> {
    entity: &'a Type,
    param: Option<&'a Ident>,
    /// Multi-param closure context for join scenarios, e.g. `|a: Blog, b: Post| ...`.
    /// Empty for single-entity forms (A/B/C non-join).
    params: Vec<(Ident, Type)>,
}

impl<'a> LinqCtx<'a> {
    fn single(entity: &'a Type, param: Option<&'a Ident>) -> Self {
        Self {
            entity,
            param,
            params: Vec::new(),
        }
    }

    /// Multi-param context for join clauses. `primary` is the param being matched;
    /// `all_params` is the full list for cross-param resolution.
    fn multi(entity: &'a Type, primary: &'a Ident, all_params: &[(Ident, Type)]) -> Self {
        Self {
            entity,
            param: Some(primary),
            params: all_params
                .iter()
                .map(|(i, t)| (i.clone(), t.clone()))
                .collect(),
        }
    }
}

/// Whether a field reference should resolve to a `COLUMN_*` (scalar) or `FIELD_*`
/// (navigation) constant on the entity type.
enum FieldKind {
    Column,
    Navigation,
}

/// A resolved field reference: the owning entity type and the field's bare name.
struct FieldRef {
    entity: Type,
    field_name: String,
}

// ---------------------------------------------------------------------------
// compile_expr — Form A/B where body → QueryBuilder method chain
// ---------------------------------------------------------------------------

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

/// `b.url.contains("x")` → LIKE; `ids.contains(b.id)` → IN (LINQ style).
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

// ---------------------------------------------------------------------------
// Field extraction
// ---------------------------------------------------------------------------

/// Extracts a `COLUMN_*` constant reference for a scalar field.
fn extract_field(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    let field_ref = extract_field_ref(ctx, expr)?;
    Ok(field_const(
        &field_ref.entity,
        &field_ref.field_name,
        FieldKind::Column,
    ))
}

/// Extracts a `FIELD_*` constant reference for a navigation field.
fn extract_field_nav(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2> {
    let field_ref = extract_field_ref(ctx, expr)?;
    Ok(field_const(
        &field_ref.entity,
        &field_ref.field_name,
        FieldKind::Navigation,
    ))
}

/// Resolves a field-access expression to its owning entity type + bare field name.
/// Recognizes `b.field` (closure param), `Blog::field` (entity path), and bare `field`.
/// For multi-param (join) contexts, resolves the owning entity from the param binding.
fn extract_field_ref(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<FieldRef> {
    match expr {
        Expr::Path(ExprPath { path, .. }) if path.segments.len() == 1 => {
            let field = path.segments[0].ident.to_string();
            Ok(FieldRef {
                entity: ctx.entity.clone(),
                field_name: field,
            })
        }
        Expr::Field(ExprField { base, member, .. }) => {
            let field_name = match member {
                Member::Named(name) => name.to_string(),
                _ => return Err(syn::Error::new_spanned(expr, "expected field name")),
            };
            if let Expr::Path(ExprPath { path, .. }) = &**base {
                // `b.field` where `b` is a closure param (single or multi).
                if path.segments.len() == 1 {
                    let base_ident = &path.segments[0].ident;
                    // Single-param form: matches ctx.param.
                    if let Some(param) = ctx.param {
                        if param == base_ident {
                            return Ok(FieldRef {
                                entity: ctx.entity.clone(),
                                field_name,
                            });
                        }
                    }
                    // Multi-param form: look up in ctx.params (join scenario).
                    if let Some((_, entity)) = ctx.params.iter().find(|(p, _)| p == base_ident) {
                        return Ok(FieldRef {
                            entity: entity.clone(),
                            field_name,
                        });
                    }
                }
                // `Blog::field` form.
                if type_path_matches(path, ctx.entity) {
                    return Ok(FieldRef {
                        entity: ctx.entity.clone(),
                        field_name,
                    });
                }
                // Multi-param: `Post::field` form matching one of the join entities.
                for (_, entity) in &ctx.params {
                    if type_path_matches(path, entity) {
                        return Ok(FieldRef {
                            entity: entity.clone(),
                            field_name,
                        });
                    }
                }
            }
            // Fallback: attribute to the primary entity.
            Ok(FieldRef {
                entity: ctx.entity.clone(),
                field_name,
            })
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

/// Generates `Entity::COLUMN_<UPPER>` or `Entity::FIELD_<UPPER>` depending on `kind`.
fn field_const(entity: &Type, field: &str, kind: FieldKind) -> TokenStream2 {
    let prefix = match kind {
        FieldKind::Column => "COLUMN_",
        FieldKind::Navigation => "FIELD_",
    };
    let const_name = Ident::new(
        &format!("{}{}", prefix, field.to_uppercase()),
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
