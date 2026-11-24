//! Aggregate / comparison operators shared by projections and HAVING.
//!
//! 2.9 removes the dedicated `HavingExpr` AST: a HAVING condition is a
//! [`BoolExpr`](super::ast::BoolExpr) whose `Compare` node holds
//! [`SqlExpr`](super::expr::SqlExpr) operands. [`AggKind`] and [`CompareOp`]
//! stay public because the `linq!(having ...)` expansion and the
//! `SelectItem`/`BoolExpr` constructors use them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggKind {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl AggKind {
    /// Returns the SQL keyword for this aggregate.
    pub fn sql_name(&self) -> &'static str {
        match self {
            AggKind::Count => "COUNT",
            AggKind::Sum => "SUM",
            AggKind::Avg => "AVG",
            AggKind::Min => "MIN",
            AggKind::Max => "MAX",
        }
    }

    /// Parses an aggregate name (case-insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "COUNT" => Some(AggKind::Count),
            "SUM" => Some(AggKind::Sum),
            "AVG" => Some(AggKind::Avg),
            "MIN" => Some(AggKind::Min),
            "MAX" => Some(AggKind::Max),
            _ => None,
        }
    }
}

/// Comparison operator for aggregate WHERE/HAVING comparisons and `BoolExpr::Compare`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl CompareOp {
    /// Returns the SQL operator string.
    pub fn sql_name(&self) -> &'static str {
        match self {
            CompareOp::Eq => "=",
            CompareOp::Ne => "!=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
        }
    }

    /// Parses a comparison operator from its SQL symbol.
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        match symbol {
            "=" => Some(CompareOp::Eq),
            "!=" => Some(CompareOp::Ne),
            ">" => Some(CompareOp::Gt),
            ">=" => Some(CompareOp::Ge),
            "<" => Some(CompareOp::Lt),
            "<=" => Some(CompareOp::Le),
            _ => None,
        }
    }
}

/// Compatibility alias: HAVING conditions are [`BoolExpr`](super::ast::BoolExpr)
/// nodes (specifically `BoolExpr::Compare`). The former dedicated enum was
/// removed in 2.9; construct conditions with
/// [`BoolExpr::aggregate_compare`](super::ast::BoolExpr::aggregate_compare).
pub type HavingExpr = super::ast::BoolExpr;
