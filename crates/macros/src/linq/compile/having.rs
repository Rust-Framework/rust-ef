//! `HAVING` expression compilation — converts `HavingExprAst` into Rust code
//! that constructs the corresponding `rust_ef::query::HavingExpr` runtime value.

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::Ident;

use super::super::ast::HavingExprAst;
use super::super::context::{extract_field, extract_value, LinqCtx};

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
