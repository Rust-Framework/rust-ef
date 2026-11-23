//! `syn::Parse` implementation for `LinqClause` — each `;`-separated clause
//! in a query (the `;`-separated clause list).

use syn::{parse::Parse, BinOp, Expr, Ident, Token, Type};

use super::super::ast::*;
use super::complex::{expr_to_having_ast, parse_window_rest, parse_with_rest};
use super::input::parse_field_or_tuple;

// ---------------------------------------------------------------------------
// LinqClause — per-clause dispatch
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
            "sum_i64" => {
                let field: Expr = input.parse()?;
                Ok(LinqClause::SumI64(field))
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
            "inner_join" => parse_join_rest(input, LinqJoinKind::Inner),
            "left_join" => parse_join_rest(input, LinqJoinKind::Left),
            "right_join" => parse_join_rest(input, LinqJoinKind::Right),
            "full_join" => parse_join_rest(input, LinqJoinKind::Full),
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

/// `from <name>` — query from a CTE name or named table source.
fn parse_from_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let name: Ident = input.parse()?;
    Ok(LinqClause::From {
        name: name.to_string(),
    })
}

/// `set b.col, value`
fn parse_set_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let field: Expr = input.parse()?;
    let _comma: Token![,] = input.parse()?;
    let value: Expr = input.parse()?;
    Ok(LinqClause::Set { field, value })
}

/// `inner_join |a: T1, b: T2| a.col == b.col` / `&&`-chained composite key
/// (and left/right/full variants).
///
/// The condition splits into one or more `(left, right)` equality pairs. The
/// left side of each pair must reference the first closure parameter and the
/// right side the second (`a.x == b.x`); reversed pairs are normalized so the
/// generated ON clause always goes `left_table.column = joined_table.column`.
fn parse_join_rest(input: syn::parse::ParseStream, kind: LinqJoinKind) -> syn::Result<LinqClause> {
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
    let keys = parse_join_keys(&cond)?;

    Ok(LinqClause::Join { kind, params, keys })
}

/// Flattens a join condition into `(left, right)` equality pairs.
///
/// Accepts `a.col == b.col` and `&&`-combined chains of them, with optional
/// parentheses: `(a.x == b.x && a.y == b.y) && a.z == b.z`.
fn parse_join_keys(cond: &Expr) -> syn::Result<Vec<(Expr, Expr)>> {
    match cond {
        Expr::Paren(paren) => parse_join_keys(&paren.expr),
        Expr::Binary(binary) if matches!(binary.op, BinOp::And(_)) => {
            let mut keys = parse_join_keys(&binary.left)?;
            keys.extend(parse_join_keys(&binary.right)?);
            Ok(keys)
        }
        Expr::Binary(binary) if matches!(binary.op, BinOp::Eq(_)) => {
            Ok(vec![((*binary.left).clone(), (*binary.right).clone())])
        }
        _ => Err(syn::Error::new_spanned(
            cond,
            "join condition must be `a.col == b.col` or an `&&` chain of such equalities \
             (composite key), e.g. `|a: Order, b: Shipment| a.id == b.order_id && a.region_id == b.region_id`",
        )),
    }
}

/// `cross_join b: T2` — no ON condition
fn parse_cross_join_rest(input: syn::parse::ParseStream) -> syn::Result<LinqClause> {
    let param: Ident = input.parse()?;
    let _colon: Token![:] = input.parse()?;
    let entity: Type = input.parse()?;
    Ok(LinqClause::CrossJoin { param, entity })
}
