//! `syn::Parse` implementations for the `linq!` macro input grammar.
//!
//! Handles three syntactic forms:
//! - **Form A**: `|b: T| body [=> order]` — reusable filter closure
//! - **Form B**: `source, |b: T| body; clause; clause; ...` — multi-clause query
//! - **Form C**: `filter |b: T| body` / `index |b: T| fields` / `key |b: T| fields` — value-producing
//!
//! `LinqInput` dispatches between query (A/B) and value (C) forms.
//! `LinqClause` parses each `;`-separated clause in Form B.

use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{
    parse::Parse, BinOp, Expr, ExprBinary, ExprPath, ExprUnary, Ident, Token, Type, UnOp,
};

use super::ast::*;

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

    Ok(ValueInput::Filter {
        entity,
        param,
        body,
    })
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
                // G4: Allow both typed (`|b: T| body`) and untyped (`|b| body`)
                // closures. For untyped, the entity type is inferred from the
                // source expression's turbofish (e.g. `ctx.set::<Blog>()`).
                // If the source has no turbofish (e.g. `db_set.query()`), fall
                // back to requiring a typed closure.
                let source_entity = source_entity_type(&first).ok();
                let (entity, where_param, where_body) = match source_entity {
                    Some(ref se) => parse_closure_with_inference(input, se)?,
                    None => parse_typed_closure(input)?,
                };
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
/// Handles `ctx.set::<Blog>()`, `ctx.set::<Blog>().query()`, and bare type paths.
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

/// G4: Parses a closure that may be typed (`|b: T| body`) or untyped
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

/// Parses `|param| body` — returns (param, body). Entity type inferred from context.
fn parse_untyped_closure(input: syn::parse::ParseStream) -> syn::Result<(Ident, Expr)> {
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
            "inner_join" => parse_join_rest(input, JoinKind::Inner),
            "left_join" => parse_join_rest(input, JoinKind::Left),
            "right_join" => parse_join_rest(input, JoinKind::Right),
            "full_join" => parse_join_rest(input, JoinKind::Full),
            "cross_join" => parse_cross_join_rest(input),
            "union" => {
                let expr: Expr = input.parse()?;
                Ok(LinqClause::Union(expr))
            }
            "union_all" => {
                let expr: Expr = input.parse()?;
                Ok(LinqClause::UnionAll(expr))
            }
            "intersect" => {
                let expr: Expr = input.parse()?;
                Ok(LinqClause::Intersect(expr))
            }
            "except" => {
                let expr: Expr = input.parse()?;
                Ok(LinqClause::Except(expr))
            }
            "execute_update" => Ok(LinqClause::ExecuteUpdate),
            "take" => {
                let n: Expr = input.parse()?;
                Ok(LinqClause::Take(n))
            }
            "skip" => {
                let n: Expr = input.parse()?;
                Ok(LinqClause::Skip(n))
            }
            "window" => parse_window_rest(input),
            "with" => parse_with_rest(input),
            "from" => parse_from_rest(input),
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

/// `having <expr>` — parses a boolean expression tree of aggregate comparisons.
///
/// Supported forms:
/// - `agg(col) op value` (e.g. `count(b.id) > 1`)
/// - `agg(col) op agg(col)` (e.g. `count(b.id) > sum(b.views)`)
/// - `expr && expr`, `expr || expr`, `!expr`, `(expr)`
fn parse_having_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let expr: Expr = input.parse()?;
    let ast = expr_to_having_ast(&expr)?;
    Ok(LinqClause::HavingExpr { expr: ast })
}

/// `window <func> [<col>] [partition_by <cols>] [order_by <col> [asc|desc]] as <alias>`
///
/// Parsing strategy: collect all tokens until end of clause (the `;` separator
/// is already stripped by `collect_until_semi`), then parse piece by piece.
///
/// Examples:
///   `window row_number partition_by b.dept_id order_by b.salary desc as rn`
///   `window sum b.salary partition_by b.dept_id as dept_total`
///   `window lag b.salary order_by b.hire_date as prev_salary`
fn parse_window_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    // 1. Function name (ident).
    let func_ident: Ident = input.parse()?;
    let func = func_ident.to_string();

    // Determine if this function takes a column argument.
    let takes_column = !matches!(
        func.to_uppercase().as_str(),
        "ROW_NUMBER" | "RANK" | "DENSE_RANK"
    );

    // 2. Optional column argument (for aggregate/offset functions).
    let column: Option<Expr> = if takes_column {
        // Peek: if the next token is `partition_by`, `order_by`, or `as`,
        // there's no column (error for non-ranking functions, but let the
        // runtime panic handle it). Otherwise parse the column expression
        // using a restricted parser that won't consume `as` as a cast.
        if is_window_keyword(input) {
            None
        } else {
            Some(parse_window_field_expr(input)?)
        }
    } else {
        None
    };

    // 3. Optional partition_by / order_by (in that order).
    let mut partition_by: Vec<Expr> = Vec::new();
    let mut order_by: Vec<(Expr, bool)> = Vec::new();

    loop {
        if input.is_empty() {
            break;
        }
        // `as` is a Rust keyword — check for it via Token![as] first.
        if input.peek(Token![as]) {
            break;
        }
        let cursor = input.cursor();
        let (ident, _) = cursor.ident().ok_or_else(|| {
            syn::Error::new(
                cursor.span(),
                "expected `partition_by`, `order_by`, or `as`",
            )
        })?;
        match ident.to_string().as_str() {
            "partition_by" => {
                let _: Ident = input.parse()?;
                partition_by = parse_window_field_list(input)?;
            }
            "order_by" => {
                let _: Ident = input.parse()?;
                order_by = parse_window_order_list(input)?;
            }
            other => {
                return Err(syn::Error::new(
                    ident.span(),
                    format!("expected `partition_by`, `order_by`, or `as`, got `{other}`"),
                ));
            }
        }
    }

    // 4. `as <alias>` — `as` is a Rust keyword, parsed via Token![as].
    let _: Token![as] = input.parse()?;
    let alias_ident: Ident = input.parse()?;
    let alias = alias_ident.to_string();

    Ok(LinqClause::Window {
        func,
        column,
        partition_by,
        order_by,
        alias,
    })
}

/// Returns `true` if the next token is a window-clause keyword
/// (`partition_by`, `order_by`, or the `as` keyword).
fn is_window_keyword(input: syn::parse::ParseStream) -> bool {
    // `as` is a Rust keyword.
    if input.peek(Token![as]) {
        return true;
    }
    if !input.peek(Ident) {
        return false;
    }
    let cursor = input.cursor();
    if let Some((ident, _)) = cursor.ident() {
        matches!(ident.to_string().as_str(), "partition_by" | "order_by")
    } else {
        false
    }
}

/// Parses a comma-separated list of field expressions, stopping at
/// `order_by` or `as`.
fn parse_window_field_list(input: syn::parse::ParseStream) -> syn::Result<Vec<Expr>> {
    let mut fields = Vec::new();
    loop {
        if input.is_empty() || is_window_keyword(input) {
            break;
        }
        let expr = parse_window_field_expr(input)?;
        fields.push(expr);
        if input.is_empty() || is_window_keyword(input) {
            break;
        }
        // Expect comma between fields.
        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else {
            break;
        }
    }
    Ok(fields)
}

/// Parses a comma-separated list of `field [asc|desc]` pairs, stopping at
/// `as`.
fn parse_window_order_list(input: syn::parse::ParseStream) -> syn::Result<Vec<(Expr, bool)>> {
    let mut pairs = Vec::new();
    loop {
        if input.is_empty() || is_window_keyword(input) {
            break;
        }
        let expr = parse_window_field_expr(input)?;
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
        pairs.push((expr, descending));
        if input.is_empty() || is_window_keyword(input) {
            break;
        }
        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else {
            break;
        }
    }
    Ok(pairs)
}

/// Parses a field-path expression (`e.field` or `e.field.sub`) without
/// consuming `as` (which `Expr::parse` would interpret as a cast).
///
/// Collects tokens until a window-clause boundary (`as`, `,`,
/// `partition_by`, `order_by`) or EOF, then parses the collected tokens
/// as an `Expr`.
fn parse_window_field_expr(input: syn::parse::ParseStream) -> syn::Result<Expr> {
    let mut tokens = TokenStream2::new();
    while !input.is_empty()
        && !input.peek(Token![as])
        && !input.peek(Token![,])
        && !is_window_field_boundary(input)
    {
        let tt: TokenTree = input.parse()?;
        tokens.extend(std::iter::once(tt));
    }
    if tokens.is_empty() {
        return Err(syn::Error::new(
            input.span(),
            "expected a field expression in window clause",
        ));
    }
    syn::parse2(tokens)
}

/// Returns `true` if the next token is a field-list boundary keyword
/// (`partition_by`, `order_by`, `asc`, or `desc`).
fn is_window_field_boundary(input: syn::parse::ParseStream) -> bool {
    if !input.peek(Ident) {
        return false;
    }
    let cursor = input.cursor();
    if let Some((ident, _)) = cursor.ident() {
        matches!(
            ident.to_string().as_str(),
            "partition_by" | "order_by" | "asc" | "desc"
        )
    } else {
        false
    }
}

/// `with <name> as |param: Type| <body>`
///
/// Defines a typed CTE whose WHERE clause is compiled from the closure body
/// into a `BoolExpr` at expansion time. The CTE body
/// `SELECT * FROM <table> WHERE <expr>` is generated at `to_sql_with` time
/// with provider-correct placeholders (`?` or `$N`).
///
/// Examples:
///   `with high_earners as |e: Employee| e.salary > 85000`
///   `with active_users as |u: User| u.active == true && u.age > 18`
fn parse_with_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    // 0. Optional `recursive` keyword.
    let recursive = if input.peek(Ident) {
        let cursor = input.cursor();
        if let Some((ident, _)) = cursor.ident() {
            if ident == "recursive" {
                let _: Ident = input.parse()?;
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    // 1. CTE name.
    let name: Ident = input.parse()?;

    // 2. `as` keyword (Rust keyword — parse via Token![as]).
    let _: Token![as] = input.parse()?;

    // 3. Typed closure: |param: Type| body
    let _open: Token![|] = input.parse()?;
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    let _close: Token![|] = input.parse()?;

    // Parse body until `;` (the clause separator is already stripped by
    // `collect_until_semi`, but we still stop at `;` for safety).
    let body = parse_expr_until_fat_arrow_or_semi(input)?;

    // 4. Optional `link <fk> to <pk>` (only for recursive CTEs).
    let link = if recursive && input.peek(Ident) {
        let cursor = input.cursor();
        if let Some((ident, _)) = cursor.ident() {
            if ident == "link" {
                let _: Ident = input.parse()?; // consume `link`
                let fk: Expr = input.parse()?;
                let to_kw: Ident = input.parse()?; // consume `to`
                if to_kw != "to" {
                    return Err(syn::Error::new(to_kw.span(), "expected `to`"));
                }
                let pk: Expr = input.parse()?;
                Some((fk, pk))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    Ok(LinqClause::With {
        name: name.to_string(),
        entity,
        param,
        body,
        recursive,
        link,
    })
}

/// `from <name>` — query from a CTE name or named table source.
fn parse_from_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let name: Ident = input.parse()?;
    Ok(LinqClause::From {
        name: name.to_string(),
    })
}

/// Converts a parsed `syn::Expr` into a `HavingExprAst`.
///
/// Walks the expression tree recursively, handling `&&`, `||`, `!`, and
/// parentheses as boolean combinators, and `agg(col) op <rhs>` as comparisons.
fn expr_to_having_ast(expr: &Expr) -> syn::Result<HavingExprAst> {
    match expr {
        Expr::Binary(b) => match &b.op {
            BinOp::And(_) => {
                let left = expr_to_having_ast(&b.left)?;
                let right = expr_to_having_ast(&b.right)?;
                Ok(HavingExprAst::And(Box::new(left), Box::new(right)))
            }
            BinOp::Or(_) => {
                let left = expr_to_having_ast(&b.left)?;
                let right = expr_to_having_ast(&b.right)?;
                Ok(HavingExprAst::Or(Box::new(left), Box::new(right)))
            }
            BinOp::Eq(_)
            | BinOp::Ne(_)
            | BinOp::Gt(_)
            | BinOp::Ge(_)
            | BinOp::Lt(_)
            | BinOp::Le(_) => parse_having_compare_from_binary(b),
            _ => Err(syn::Error::new_spanned(
                expr,
                "having expression supports only `&&`, `||`, `!`, and comparison operators",
            )),
        },
        Expr::Unary(ExprUnary {
            op: UnOp::Not(_),
            expr: inner,
            ..
        }) => {
            let inner_ast = expr_to_having_ast(inner)?;
            Ok(HavingExprAst::Not(Box::new(inner_ast)))
        }
        Expr::Paren(p) => expr_to_having_ast(&p.expr),
        _ => Err(syn::Error::new_spanned(
            expr,
            "having expects a boolean expression of aggregate comparisons",
        )),
    }
}

/// Parses `agg(col) op <rhs>` from a binary expression.
///
/// `<rhs>` is either a literal/value (→ `Compare`) or another `agg(col)` call
/// (→ `CompareAgg`).
fn parse_having_compare_from_binary(b: &ExprBinary) -> syn::Result<HavingExprAst> {
    let op = bin_op_to_symbol(&b.op)?;
    let (left_agg, left_col) = parse_agg_call(&b.left)?;

    // Try to parse the right side as an aggregate call. If that succeeds,
    // it's a `CompareAgg`; otherwise treat it as a value.
    match parse_agg_call(&b.right) {
        Ok((right_agg, right_col)) => Ok(HavingExprAst::CompareAgg {
            left_agg,
            left_col,
            op: op.to_string(),
            right_agg,
            right_col,
        }),
        Err(_) => {
            let value: Expr = (*b.right).clone();
            Ok(HavingExprAst::Compare {
                agg: left_agg,
                col: left_col,
                op: op.to_string(),
                value,
            })
        }
    }
}

/// Extracts `(agg_name_uppercase, col_expr)` from an `agg(col)` call.
///
/// Returns an error if `expr` is not a single-argument call to one of
/// `count`/`sum`/`avg`/`min`/`max`.
fn parse_agg_call(expr: &Expr) -> syn::Result<(String, Expr)> {
    let call = match expr {
        Expr::Call(c) => c,
        _ => {
            return Err(syn::Error::new_spanned(
                expr,
                "expected aggregate function call, e.g. `count(b.id)`",
            ));
        }
    };
    let agg = match &*call.func {
        Expr::Path(p) if p.path.segments.len() == 1 => p.path.segments[0].ident.to_string(),
        _ => {
            return Err(syn::Error::new_spanned(
                &call.func,
                "expected aggregate function: count/sum/avg/min/max",
            ));
        }
    };
    // Validate aggregate name.
    if !matches!(
        agg.to_lowercase().as_str(),
        "count" | "sum" | "avg" | "min" | "max"
    ) {
        return Err(syn::Error::new_spanned(
            &call.func,
            "unsupported aggregate; use count/sum/avg/min/max",
        ));
    }
    let col = call
        .args
        .first()
        .ok_or_else(|| {
            syn::Error::new_spanned(&call.func, "aggregate function requires a column argument")
        })?
        .clone();
    Ok((agg.to_uppercase(), col))
}

/// Maps a `syn::BinOp` comparison variant to its SQL symbol.
fn bin_op_to_symbol(op: &BinOp) -> syn::Result<&'static str> {
    match op {
        BinOp::Eq(_) => Ok("="),
        BinOp::Ne(_) => Ok("!="),
        BinOp::Gt(_) => Ok(">"),
        BinOp::Ge(_) => Ok(">="),
        BinOp::Lt(_) => Ok("<"),
        BinOp::Le(_) => Ok("<="),
        _ => Err(syn::Error::new_spanned(
            op,
            "unsupported comparison operator in having",
        )),
    }
}

/// `set b.col, value`
fn parse_set_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let field: Expr = input.parse()?;
    let _comma: Token![,] = input.parse()?;
    let value: Expr = input.parse()?;
    Ok(LinqClause::Set { field, value })
}

/// Join kind for `parse_join_rest`.
#[derive(Clone, Copy)]
enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
}

/// `inner_join |a: T1, b: T2| a.col == b.col` (and left/right/full variants)
fn parse_join_rest(input: syn::parse::ParseStream, kind: JoinKind) -> syn::Result<LinqClause> {
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

    Ok(match kind {
        JoinKind::Inner => LinqClause::InnerJoin {
            params,
            left,
            right,
        },
        JoinKind::Left => LinqClause::LeftJoin {
            params,
            left,
            right,
        },
        JoinKind::Right => LinqClause::RightJoin {
            params,
            left,
            right,
        },
        JoinKind::Full => LinqClause::FullJoin {
            params,
            left,
            right,
        },
    })
}

/// `cross_join b: T2` — no ON condition
fn parse_cross_join_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    Ok(LinqClause::CrossJoin { param, entity })
}
