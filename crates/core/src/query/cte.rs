//! Common Table Expression (CTE) and SQL set operation specifications.
//!
//! `CteSpec` supports both raw-mode (pre-compiled SQL) and typed-mode
//! (compile-from-`BoolExpr`) CTEs, including recursive CTEs that emit
//! `WITH RECURSIVE ... UNION ALL SELECT ... JOIN name ON ...`.
//!
//! `SetOperator` / `SetOpSpec` represent UNION / INTERSECT / EXCEPT
//! operands appended after the main SELECT.

use crate::provider::DbValue;

use super::ast::BoolExpr;

/// Specification for a Common Table Expression (CTE).
///
/// A CTE is defined by a name and either a pre-compiled SQL string (raw mode)
/// or a typed WHERE expression compiled at SQL generation time (typed mode).
/// The main query references the CTE by name (typically in its FROM clause).
/// Parameters are prepended to the main query's parameter list in CTE
/// declaration order.
///
/// ## Modes
///
/// - **Raw mode** (via `with_cte_internal`): `sql` is non-empty, `table` and
///   `where_expr` are empty. The SQL is emitted verbatim. Placeholders use
///   the `?` style and are **not** converted to provider-specific syntax —
///   suitable for SQLite/MySQL but may produce incorrect `$N` on PostgreSQL.
///
/// - **Typed mode** (via `with_cte_typed`, used by `linq!(with ...)`): `table`
///   is non-empty, `where_expr` is `Some(...)`, `sql` is empty. The CTE body
///   `SELECT * FROM <table> WHERE <expr>` is compiled at `to_sql_with` time
///   using the provider's placeholder syntax, ensuring correct `$N` numbering
///   on all providers.
///
/// `#[non_exhaustive]` prevents direct struct construction outside the crate
/// so future field additions don't break downstream code. Use
/// `with_cte_internal` (raw mode) or `with_cte_typed` (typed mode) to create
/// CTE specifications.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CteSpec {
    /// The CTE name (used as the derived table alias in `WITH name AS (...)`).
    pub name: String,
    /// Raw mode: the pre-compiled SQL of the CTE body. Empty in typed mode.
    pub sql: String,
    /// Typed mode: source table name (`SELECT * FROM <table> WHERE ...`).
    /// Empty in raw mode.
    pub table: String,
    /// Typed mode: WHERE expression compiled at `to_sql_with` time with the
    /// provider's placeholder syntax. `None` in raw mode.
    pub where_expr: Option<BoolExpr>,
    /// Parameter values bound to the CTE's placeholders, in order.
    /// In typed mode, these are extracted from `where_expr` via
    /// `collect_bool_expr_values` at construction time.
    pub params: Vec<DbValue>,
    /// Optional explicit column list (`WITH name (c1, c2) AS (...)`).
    /// Empty means no explicit column list.
    pub columns: Vec<String>,
    /// Recursive CTE flag. When true, generates
    /// `WITH RECURSIVE name AS (anchor UNION ALL SELECT t.* FROM table t JOIN name ON t.fk = name.pk)`.
    pub is_recursive: bool,
    /// Recursive link columns: `(fk_column, pk_column)`. Only meaningful when
    /// `is_recursive` is true. The recursive member joins the CTE name to the
    /// source table via `t.fk = name.pk`.
    pub recursive_link: Option<(String, String)>,
}

/// SQL set operators for combining result sets (UNION / INTERSECT / EXCEPT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOperator {
    Union,
    UnionAll,
    Intersect,
    Except,
}

/// A set operation operand: a pre-compiled SQL string and its bound params.
///
/// Per D5, operands should not contain ORDER BY / LIMIT (caller responsibility).
#[derive(Debug, Clone)]
pub struct SetOpSpec {
    pub operator: SetOperator,
    pub operand_sql: String,
    pub operand_params: Vec<DbValue>,
}
