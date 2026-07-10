//! `syn::Parse` implementation for `LinqClause` — each `;`-separated clause
//! in a Form B multi-clause query.

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
