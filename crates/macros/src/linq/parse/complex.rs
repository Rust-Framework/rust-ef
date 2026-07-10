//! Complex clause parsing: window functions, CTE/with, and having AST conversion.

use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use syn::{BinOp, Expr, ExprBinary, ExprUnary, Ident, Token, Type, UnOp};

use super::super::ast::*;
use super::input::parse_expr_until_fat_arrow_or_semi;

// ---------------------------------------------------------------------------
// Window clause
// ---------------------------------------------------------------------------

/// `window <func> [<col>] [partition_by <cols>] [order_by <col> [asc|desc]] as <alias>`
///
/// Parsing strategy: collect all tokens until end of clause (the `;` separator
/// is already stripped by `collect_until_semi`), then parse piece by piece.
///
/// Examples:
///   `window row_number partition_by b.dept_id order_by b.salary desc as rn`
///   `window sum b.salary partition_by b.dept_id as dept_total`
///   `window lag b.salary order_by b.hire_date as prev_salary`
pub(super) fn parse_window_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
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

// ---------------------------------------------------------------------------
// CTE / with clause
// ---------------------------------------------------------------------------

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
pub(super) fn parse_with_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
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

// ---------------------------------------------------------------------------
// Having AST conversion
// ---------------------------------------------------------------------------

/// Converts a parsed `syn::Expr` into a `HavingExprAst`.
///
/// Walks the expression tree recursively, handling `&&`, `||`, `!`, and
/// parentheses as boolean combinators, and `agg(col) op <rhs>` as comparisons.
pub(super) fn expr_to_having_ast(expr: &Expr) -> syn::Result<HavingExprAst> {
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
