//! Typed projection expressions (`SqlExpr`, `SelectItem`).
//!
//! `SqlExpr` is the provider-neutral expression AST behind typed report
//! projections (`SelectItem`). It compiles with the provider's
//! `ISqlGenerator` placeholder syntax and collects bound values in exactly the
//! same left-to-right order as it emits placeholders, which is what keeps
//! `QueryState::all_params()` aligned with the generated SQL.
//!
//! Scope by design: `CASE` conditions are [`FilterCondition`]s
//! (`col op value` / `IN` / `BETWEEN` / `IS NULL`), portable across SQLite,
//! PostgreSQL and MySQL. Subquery expressions are out of scope — use raw SQL
//! plus `IRow` (`DbContext::sql_query_rows`) for those.

use crate::provider::{DbValue, ISqlGenerator};

use super::ast::FilterCondition;

/// Aggregate function used by a [`SqlExpr::Aggregate`] node.
///
/// This is the same type as [`AggKind`](super::having_expr::AggKind): HAVING
/// expressions and SELECT-list projections share one aggregate enum (with
/// `sql_name` / `from_name`), so the SQL names cannot drift between the two
/// layers.
pub use super::having_expr::AggKind as AggregateFunc;

/// Arithmetic operator for a [`SqlExpr::Binary`] node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
}

impl ArithmeticOp {
    /// Returns the SQL operator symbol.
    pub fn sql_operator(&self) -> &'static str {
        match self {
            ArithmeticOp::Add => "+",
            ArithmeticOp::Sub => "-",
        }
    }
}

/// One `WHEN <condition> THEN <result>` arm of a [`SqlExpr::Case`].
#[derive(Debug, Clone)]
pub struct CaseArm {
    /// The `WHEN` condition (a parameterized filter condition).
    pub condition: FilterCondition,
    /// The `THEN` result expression.
    pub result: SqlExpr,
}

/// A provider-neutral SQL expression used in SELECT projections.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum SqlExpr {
    /// A bare column name, or a `table.column` reference emitted verbatim.
    ///
    /// The string is SQL identifier text, not a value: it must come from code
    /// constants / a whitelist, never from request data (values belong in
    /// [`SqlExpr::Literal`] or a `FilterCondition`, which bind parameters).
    Column(String),
    /// An aggregate call, e.g. `SUM(amount)` / `COUNT(*)`.
    Aggregate {
        /// The aggregate function.
        func: AggregateFunc,
        /// The aggregated column (`None` means `*`, i.e. `COUNT(*)`).
        column: Option<String>,
        /// Emit `DISTINCT` inside the aggregate call (`SUM(DISTINCT col)`).
        distinct: bool,
    },
    /// `COALESCE(<expr>, <fallback>)` — the fallback is a bound parameter.
    Coalesce(Box<SqlExpr>, DbValue),
    /// `CASE WHEN ... THEN ... [ELSE ...] END`.
    Case {
        /// The `WHEN`/`THEN` arms, emitted in order.
        arms: Vec<CaseArm>,
        /// The optional `ELSE` expression.
        otherwise: Option<Box<SqlExpr>>,
    },
    /// A bound constant.
    Literal(DbValue),
    /// A binary arithmetic expression, e.g. `(amount - discount)`.
    Binary {
        /// Left operand.
        left: Box<SqlExpr>,
        /// The operator.
        op: ArithmeticOp,
        /// Right operand.
        right: Box<SqlExpr>,
    },
}

impl SqlExpr {
    /// A bare column reference (`column` or `table.column`, emitted verbatim).
    pub fn column(column: impl Into<String>) -> Self {
        SqlExpr::Column(column.into())
    }

    /// `SUM(column)`.
    pub fn sum(column: impl Into<String>) -> Self {
        SqlExpr::aggregate(AggregateFunc::Sum, Some(column.into()), false)
    }

    /// `AVG(column)`.
    pub fn avg(column: impl Into<String>) -> Self {
        SqlExpr::aggregate(AggregateFunc::Avg, Some(column.into()), false)
    }

    /// `MIN(column)`.
    pub fn min(column: impl Into<String>) -> Self {
        SqlExpr::aggregate(AggregateFunc::Min, Some(column.into()), false)
    }

    /// `MAX(column)`.
    pub fn max(column: impl Into<String>) -> Self {
        SqlExpr::aggregate(AggregateFunc::Max, Some(column.into()), false)
    }

    /// `COUNT(*)`.
    pub fn count_all() -> Self {
        SqlExpr::aggregate(AggregateFunc::Count, None, false)
    }

    /// `COUNT(column)`.
    pub fn count_column(column: impl Into<String>) -> Self {
        SqlExpr::aggregate(AggregateFunc::Count, Some(column.into()), false)
    }

    /// `COUNT(DISTINCT column)`.
    pub fn count_distinct(column: impl Into<String>) -> Self {
        SqlExpr::aggregate(AggregateFunc::Count, Some(column.into()), true)
    }

    /// A bound constant.
    pub fn literal(value: impl Into<DbValue>) -> Self {
        SqlExpr::Literal(value.into())
    }

    /// Compatibility alias for [`SqlExpr::literal`] (the former `ColumnExpr`
    /// SET-expression constructor).
    pub fn value(value: impl Into<DbValue>) -> Self {
        SqlExpr::literal(value)
    }

    /// Wraps this expression in `COALESCE(self, fallback)`.
    pub fn coalesce(self, fallback: impl Into<DbValue>) -> Self {
        SqlExpr::Coalesce(Box::new(self), fallback.into())
    }

    /// `(self + other)`.
    // Binary expression builder; the method name intentionally mirrors the
    // operator (`SqlExpr::add`/`sub`) and does not implement `std::ops`.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: SqlExpr) -> Self {
        SqlExpr::Binary {
            left: Box::new(self),
            op: ArithmeticOp::Add,
            right: Box::new(other),
        }
    }

    /// `(self - other)`.
    #[allow(clippy::should_implement_trait)]
    pub fn sub(self, other: SqlExpr) -> Self {
        SqlExpr::Binary {
            left: Box::new(self),
            op: ArithmeticOp::Sub,
            right: Box::new(other),
        }
    }

    /// `CASE WHEN <arms> [ELSE <otherwise>] END`.
    pub fn case(arms: Vec<CaseArm>, otherwise: Option<SqlExpr>) -> Self {
        SqlExpr::Case {
            arms,
            otherwise: otherwise.map(Box::new),
        }
    }

    pub(crate) fn aggregate(func: AggregateFunc, column: Option<String>, distinct: bool) -> Self {
        SqlExpr::Aggregate {
            func,
            column,
            distinct,
        }
    }

    /// Compiles the expression and collects its bound values **in one pass**.
    ///
    /// `param_idx` is the 1-based index of the next placeholder and is advanced
    /// past every emitted placeholder; `params` receives the bound values in
    /// exactly the same left-to-right order. There is no separate
    /// "collect parameters" traversal, so SQL text order and the parameter
    /// vector cannot drift.
    pub(crate) fn compile(
        &self,
        gen: &dyn ISqlGenerator,
        param_idx: &mut usize,
        params: &mut Vec<DbValue>,
    ) -> String {
        match self {
            SqlExpr::Column(column) => column.clone(),
            SqlExpr::Aggregate {
                func,
                column,
                distinct,
            } => match column {
                Some(column) => {
                    let distinct_kw = if *distinct { "DISTINCT " } else { "" };
                    format!("{}({}{})", func.sql_name(), distinct_kw, column)
                }
                // Column-less aggregates are always `COUNT(*)`; `COUNT(DISTINCT *)`
                // is not valid SQL, so `distinct` is ignored here (the builder
                // reports a configuration error for `SELECT DISTINCT` + COUNT).
                None => format!("{}(*)", func.sql_name()),
            },
            SqlExpr::Coalesce(expr, fallback) => {
                let expr_sql = expr.compile(gen, param_idx, params);
                let placeholder = gen.parameter_placeholder(*param_idx);
                *param_idx += 1;
                params.push(fallback.clone());
                format!("COALESCE({expr_sql}, {placeholder})")
            }
            SqlExpr::Case { arms, otherwise } => {
                let mut sql = String::from("CASE");
                for arm in arms {
                    sql.push_str(" WHEN ");
                    sql.push_str(&arm.condition.compile(gen, param_idx, params));
                    sql.push_str(" THEN ");
                    sql.push_str(&arm.result.compile(gen, param_idx, params));
                }
                if let Some(otherwise) = otherwise {
                    sql.push_str(" ELSE ");
                    sql.push_str(&otherwise.compile(gen, param_idx, params));
                }
                sql.push_str(" END");
                sql
            }
            SqlExpr::Literal(value) => {
                let placeholder = gen.parameter_placeholder(*param_idx);
                *param_idx += 1;
                params.push(value.clone());
                placeholder
            }
            SqlExpr::Binary { left, op, right } => format!(
                "({} {} {})",
                left.compile(gen, param_idx, params),
                op.sql_operator(),
                right.compile(gen, param_idx, params)
            ),
        }
    }

    /// Number of bound values carried by this expression.
    pub(crate) fn param_count(&self) -> usize {
        let mut index = 1usize;
        let mut params = Vec::new();
        let _ = self.compile(
            &crate::query::compile::PortablePlaceholderGenerator,
            &mut index,
            &mut params,
        );
        params.len()
    }
}

/// A logical end segment of a (possibly qualified) column name.
fn last_segment(column: &str) -> &str {
    column.rsplit('.').next().unwrap_or(column)
}

/// Debug-only validation for runtime identifier strings.
///
/// The runtime builder accepts identifiers as `&str` (EF Core has expression
/// trees; rust-ef's runtime API is the expert surface). A caller that passes a
/// request value by mistake must fail loudly in development instead of
/// producing an injectable statement: this assertion only fires in debug builds
/// and accepts a dotted identifier (`table.column`).
pub(crate) fn debug_assert_identifier(text: &str, context: &str) {
    debug_assert!(
        !text.is_empty()
            && text.split('.').all(|part| {
                let mut chars = part.chars();
                matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
                    && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            }),
        "{context} must be an SQL identifier (`col` or `table.col`), got {text:?}; \n         runtime identifier APIs must never receive request data"
    );
}

/// Debug-only validation for a plain runtime column list.
pub(crate) fn debug_assert_identifier_columns(columns: &[&str], context: &str) {
    debug_assert!(!columns.is_empty(), "{context} needs at least one column");
    for column in columns {
        debug_assert_identifier(column, context);
    }
}

/// Validates that `expr` only uses the SET-expression subset of [`SqlExpr`]:
/// columns, bound literals and `+`/`-` arithmetic. Aggregates, COALESCE, CASE
/// and anything else are rejected (a `SET` value cannot aggregate).
/// Compatibility alias for [`SqlExpr`] used by SET-expression APIs.
///
/// 2.9 removed the separate `ColumnExpr` AST: a SET value is a [`SqlExpr`] that
/// is validated to the arithmetic subset (`Column`/`Literal`/`Binary` with
/// `+`/`-`) at compile time. The alias keeps the historical constructor paths
/// (`ColumnExpr::column` / `::value` / `.add` / `.sub`) working.
pub type ColumnExpr = SqlExpr;

pub(crate) fn validate_set_expression(expr: &SqlExpr) -> Result<(), String> {
    match expr {
        SqlExpr::Column(_) | SqlExpr::Literal(_) => Ok(()),
        SqlExpr::Binary { left, right, .. } => {
            validate_set_expression(left)?;
            validate_set_expression(right)
        }
        SqlExpr::Aggregate { .. } => {
            Err("aggregate functions are not allowed in a SET value".to_string())
        }
        SqlExpr::Coalesce(..) => Err("COALESCE is not allowed in a SET value".to_string()),
        SqlExpr::Case { .. } => Err("CASE is not allowed in a SET value".to_string()),
    }
}

/// One SELECT-list projection: an expression plus its output alias.
///
/// `to_sql` always emits `<expr> AS <alias>`; aliases are emitted bare (like
/// the column names used by the existing query layer).
///
/// # Identifier safety
///
/// Column names, aggregate arguments and aliases are rendered as SQL
/// identifiers **verbatim**, exactly like [`QueryBuilder`](crate::query::QueryBuilder)'s
/// ordering/`group_by` methods. They are an expert runtime API: pass entity
/// `COLUMN_*` constants or a hard-coded whitelist, never request data. For
/// request-driven queries use the compile-time-checked `linq!` surface (or map
/// untrusted values to a whitelist of column constants) and keep all values in
/// bound parameters (`SelectItem::literal` / `coalesce` are parameterised).
#[derive(Debug, Clone)]
pub struct SelectItem {
    expr: SqlExpr,
    alias: String,
}

impl SelectItem {
    /// Projects a column. The alias defaults to the column's last name segment
    /// (`"t.col"` → `"col"`).
    pub fn column(column: impl Into<String>) -> Self {
        let column = column.into();
        debug_assert_identifier(&column, "SelectItem::column");
        let alias = last_segment(&column).to_string();
        Self {
            expr: SqlExpr::Column(column),
            alias,
        }
    }

    /// Projects `SUM(column)` with alias `sum_<column>`.
    pub fn sum(column: impl Into<String>) -> Self {
        let column = column.into();
        debug_assert_identifier(&column, "SelectItem::sum");
        let alias = format!("sum_{}", last_segment(&column));
        Self {
            expr: SqlExpr::sum(column),
            alias,
        }
    }

    /// Projects `AVG(column)` with alias `avg_<column>`.
    pub fn avg(column: impl Into<String>) -> Self {
        let column = column.into();
        debug_assert_identifier(&column, "SelectItem::avg");
        let alias = format!("avg_{}", last_segment(&column));
        Self {
            expr: SqlExpr::avg(column),
            alias,
        }
    }

    /// Projects `MIN(column)` with alias `min_<column>`.
    pub fn min(column: impl Into<String>) -> Self {
        let column = column.into();
        debug_assert_identifier(&column, "SelectItem::min");
        let alias = format!("min_{}", last_segment(&column));
        Self {
            expr: SqlExpr::min(column),
            alias,
        }
    }

    /// Projects `MAX(column)` with alias `max_<column>`.
    pub fn max(column: impl Into<String>) -> Self {
        let column = column.into();
        debug_assert_identifier(&column, "SelectItem::max");
        let alias = format!("max_{}", last_segment(&column));
        Self {
            expr: SqlExpr::max(column),
            alias,
        }
    }

    /// Projects `COUNT(*)` with alias `count`.
    pub fn count() -> Self {
        Self {
            expr: SqlExpr::count_all(),
            alias: "count".to_string(),
        }
    }

    /// Projects `COUNT(column)` with alias `count_<column>`.
    pub fn count_column(column: impl Into<String>) -> Self {
        let column = column.into();
        debug_assert_identifier(&column, "SelectItem::count_column");
        let alias = format!("count_{}", last_segment(&column));
        Self {
            expr: SqlExpr::count_column(column),
            alias,
        }
    }

    /// Projects `COUNT(DISTINCT column)` with alias `count_<column>`.
    pub fn count_distinct(column: impl Into<String>) -> Self {
        let column = column.into();
        debug_assert_identifier(&column, "SelectItem::count_distinct");
        let alias = format!("count_{}", last_segment(&column));
        Self {
            expr: SqlExpr::count_distinct(column),
            alias,
        }
    }

    /// Projects a bound constant with an explicit alias, e.g.
    /// `'TENANT' AS grant_scope`.
    pub fn literal(value: impl Into<DbValue>, alias: impl Into<String>) -> Self {
        Self {
            expr: SqlExpr::literal(value),
            alias: alias.into(),
        }
    }

    /// Projects an arbitrary expression with an explicit alias (escape hatch).
    pub fn expr(expr: SqlExpr, alias: impl Into<String>) -> Self {
        Self {
            expr,
            alias: alias.into(),
        }
    }

    /// Wraps this projection in `COALESCE(<expr>, fallback)`, keeping the alias.
    ///
    /// This is the explicit way to get a neutral value for a group where the
    /// aggregate is SQL `NULL` (e.g. `SelectItem::sum("amount").coalesce(0)`).
    pub fn coalesce(self, fallback: impl Into<DbValue>) -> Self {
        Self {
            expr: self.expr.coalesce(fallback),
            alias: self.alias,
        }
    }

    /// Overrides the output alias.
    ///
    /// The alias is emitted as a bare SQL identifier: use a code constant, never
    /// request data (aliases also appear in `ORDER BY`).
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        let alias = alias.into();
        debug_assert_identifier(&alias, "SelectItem::alias");
        self.alias = alias;
        self
    }

    /// The projected expression.
    pub fn expression(&self) -> &SqlExpr {
        &self.expr
    }

    /// The output alias (column name / `sum_<column>` / override).
    pub fn alias_name(&self) -> &str {
        &self.alias
    }

    /// Compiles this projection as `<expr> AS <alias>`, collecting its bound
    /// values into `params` in placeholder order.
    pub(crate) fn compile(
        &self,
        gen: &dyn ISqlGenerator,
        param_idx: &mut usize,
        params: &mut Vec<DbValue>,
    ) -> String {
        format!(
            "{} AS {}",
            self.expr.compile(gen, param_idx, params),
            self.alias
        )
    }

    /// Collects this projection's bound values in placeholder order.
    #[cfg(test)]
    pub(crate) fn params(&self) -> Vec<DbValue> {
        let mut index = 1usize;
        let mut params = Vec::new();
        let _ = self.compile(
            &crate::query::compile::PortablePlaceholderGenerator,
            &mut index,
            &mut params,
        );
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::query::compile::PortablePlaceholderGenerator;

    /// Compiles with the portable generator and returns `(sql, params)`.
    fn compile(expr: &SqlExpr) -> (String, Vec<DbValue>) {
        let mut index = 1usize;
        let mut params = Vec::new();
        let sql = expr.compile(&PortablePlaceholderGenerator, &mut index, &mut params);
        assert_eq!(
            index - 1,
            params.len(),
            "every placeholder must collect exactly one value"
        );
        (sql, params)
    }

    #[test]
    fn column_alias_is_the_last_name_segment() {
        let item = SelectItem::column("o.amount");
        let mut index = 1usize;
        let mut params = Vec::new();
        let sql = item.compile(&PortablePlaceholderGenerator, &mut index, &mut params);
        assert_eq!(sql, "o.amount AS amount");
        assert_eq!(index, 1, "plain columns bind nothing");
        assert!(params.is_empty());
    }

    #[test]
    fn aggregate_aliases_use_the_function_prefix() {
        let gen = PortablePlaceholderGenerator;
        for (item, expected) in [
            (SelectItem::sum("o.amount"), "SUM(o.amount) AS sum_amount"),
            (SelectItem::avg("rating"), "AVG(rating) AS avg_rating"),
            (SelectItem::min("price"), "MIN(price) AS min_price"),
            (SelectItem::max("price"), "MAX(price) AS max_price"),
            (SelectItem::count(), "COUNT(*) AS count"),
            (SelectItem::count_column("b.id"), "COUNT(b.id) AS count_id"),
            (
                SelectItem::count_distinct("b.id"),
                "COUNT(DISTINCT b.id) AS count_id",
            ),
        ] {
            let mut index = 1usize;
            let mut params = Vec::new();
            assert_eq!(item.compile(&gen, &mut index, &mut params), expected);
            assert_eq!(index, 1);
        }
    }

    #[test]
    fn coalesce_projection_binds_the_fallback_after_the_expression() {
        let item = SelectItem::sum("amount").coalesce(0i64);
        assert_eq!(item.params(), vec![DbValue::I64(0)]);
    }

    #[test]
    fn literal_projection_binds_its_value() {
        let item = SelectItem::literal("TENANT", "grant_scope");
        let mut index = 1usize;
        let mut params = Vec::new();
        assert_eq!(
            item.compile(&PortablePlaceholderGenerator, &mut index, &mut params),
            "? AS grant_scope"
        );
        assert_eq!(params, vec![DbValue::String("TENANT".into())]);
    }

    #[test]
    fn case_projection_compiles_conditions_and_results_in_order() {
        let item = SelectItem::expr(
            SqlExpr::case(
                vec![CaseArm {
                    condition: FilterCondition::with_values("amount", ">", vec![DbValue::I64(100)]),
                    result: SqlExpr::literal("big"),
                }],
                Some(SqlExpr::literal("small")),
            ),
            "size",
        );
        let mut index = 1usize;
        let mut params = Vec::new();
        let sql = item.compile(&PortablePlaceholderGenerator, &mut index, &mut params);
        assert_eq!(sql, "CASE WHEN amount > ? THEN ? ELSE ? END AS size");
        assert_eq!(
            params,
            vec![
                DbValue::I64(100),
                DbValue::String("big".into()),
                DbValue::String("small".into()),
            ]
        );
        assert_eq!(index, 4);
    }

    #[test]
    fn binary_expression_compiles_with_parentheses() {
        let expr = SqlExpr::column("amount").sub(SqlExpr::literal(5i64));
        let (sql, params) = compile(&expr);
        assert_eq!(sql, "(amount - ?)");
        assert_eq!(params, vec![DbValue::I64(5)]);
        assert_eq!(expr.param_count(), 1);
    }

    #[test]
    fn alias_override_keeps_the_expression() {
        let item = SelectItem::sum("amount").alias("total");
        let mut index = 1usize;
        let mut params = Vec::new();
        assert_eq!(
            item.compile(&PortablePlaceholderGenerator, &mut index, &mut params),
            "SUM(amount) AS total"
        );
    }

    #[test]
    fn parameter_order_matches_sql_text_in_depth() {
        // Binary(Column, Add, Coalesce(Literal, Literal)) must collect the
        // literals in the exact order their placeholders appear.
        let expr = SqlExpr::column("a").add(SqlExpr::literal(1i64).coalesce(2i64));
        let (sql, params) = compile(&expr);
        assert_eq!(sql, "(a + COALESCE(?, ?))");
        assert_eq!(params, vec![DbValue::I64(1), DbValue::I64(2)]);
    }
}
