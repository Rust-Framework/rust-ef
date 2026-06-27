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
use quote::{format_ident, quote};
use syn::{
    parse::Parse, parse_macro_input, BinOp, Expr, ExprClosure, ExprField, ExprLit, ExprMethodCall,
    ExprPath, ExprUnary, Ident, Lit, Member, Pat, Token, Type, UnOp,
};

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

/// Top-level dispatch: query form (A/B) vs. value form (C).
#[allow(clippy::large_enum_variant)]
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
    Filter {
        entity: Type,
        param: Ident,
        body: Expr,
    },
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
    /// `having <expr>` — supports `agg(col) op value`, `AND`, `OR`, `NOT`,
    /// and `agg(col) op agg(col)`.
    HavingExpr { expr: HavingExprAst },
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
    InnerJoin {
        params: Vec<(Ident, Type)>,
        left: Expr,
        right: Expr,
    },
    /// `left_join |a: T1, b: T2| a.col == b.col`
    LeftJoin {
        params: Vec<(Ident, Type)>,
        left: Expr,
        right: Expr,
    },
    /// `execute_update` (terminal, triggers bulk update)
    ExecuteUpdate,
    /// `take N`
    Take(Expr),
    /// `skip N`
    Skip(Expr),
    /// `window <func> [<col>] [partition_by <cols>] [order_by <col> [asc|desc]] as <alias>`
    Window {
        func: String,
        column: Option<Expr>,
        partition_by: Vec<Expr>,
        order_by: Vec<(Expr, bool)>,
        alias: String,
    },
}

/// Macro-side AST for `HAVING` expressions.
///
/// Mirrors `rust_ef::query::HavingExpr` but carries `syn::Expr` nodes for
/// column references and values, which are resolved to column constants and
/// `DbValue` literals at expansion time by `compile_having_expr`.
#[derive(Debug)]
enum HavingExprAst {
    /// `agg(col) op value`
    Compare {
        agg: String,
        col: Expr,
        op: String,
        value: Expr,
    },
    /// `expr AND expr`
    And(Box<HavingExprAst>, Box<HavingExprAst>),
    /// `expr OR expr`
    Or(Box<HavingExprAst>, Box<HavingExprAst>),
    /// `NOT expr`
    Not(Box<HavingExprAst>),
    /// `agg(col1) op agg(col2)`
    CompareAgg {
        left_agg: String,
        left_col: Expr,
        op: String,
        right_agg: String,
        right_col: Expr,
    },
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
            "window" => parse_window_rest(input),
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
fn parse_having_compare_from_binary(b: &syn::ExprBinary) -> syn::Result<HavingExprAst> {
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
        Ok(LinqClause::LeftJoin {
            params,
            left,
            right,
        })
    } else {
        Ok(LinqClause::InnerJoin {
            params,
            left,
            right,
        })
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
fn compile_contains(
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
enum SubqueryKind {
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
fn extract_subquery_closure(closure: &ExprClosure) -> syn::Result<(Ident, Type)> {
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
fn compile_in_subquery_method(
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
fn compile_in_subquery_bool(
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

fn compile_method(ctx: &LinqCtx<'_>, call: &ExprMethodCall) -> syn::Result<TokenStream2> {
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
    path.segments.last().is_some_and(|s| s.ident == entity_name)
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

/// Compiles a `HavingExprAst` into Rust code that constructs the corresponding
/// `rust_ef::query::HavingExpr` runtime value.
///
/// Column references are resolved to `Entity::COLUMN_*` constants via
/// `extract_field`, and value literals via `extract_value`. Aggregate names
/// and operators (validated during parsing) are mapped to enum variants.
fn compile_having_expr(ast: &HavingExprAst, ctx: &LinqCtx<'_>) -> syn::Result<TokenStream2> {
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
