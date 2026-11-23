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
pub enum SqlExpr {
    /// A bare column name, or a `table.column` reference emitted verbatim.
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

    /// A bound constant.
    pub fn literal(value: impl Into<DbValue>) -> Self {
        SqlExpr::Literal(value.into())
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

    fn aggregate(func: AggregateFunc, column: Option<String>, distinct: bool) -> Self {
        SqlExpr::Aggregate {
            func,
            column,
            distinct,
        }
    }

    /// Compiles the expression using the provider's placeholder syntax.
    ///
    /// `param_idx` is advanced past every bound parameter in the same
    /// left-to-right order as [`SqlExpr::collect_params`], so callers can keep
    /// emitting contiguous placeholders (e.g. WHERE after SELECT projections).
    pub(crate) fn to_sql(&self, gen: &dyn ISqlGenerator, param_idx: &mut usize) -> String {
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
                // is not valid SQL, so `distinct` is ignored here.
                None => format!("{}(*)", func.sql_name()),
            },
            SqlExpr::Coalesce(expr, _fallback) => {
                let expr_sql = expr.to_sql(gen, param_idx);
                let placeholder = gen.parameter_placeholder(*param_idx);
                *param_idx += 1;
                format!("COALESCE({expr_sql}, {placeholder})")
            }
            SqlExpr::Case { arms, otherwise } => {
                let mut sql = String::from("CASE");
                for arm in arms {
                    let placeholders: Vec<String> = (0..arm.condition.param_count())
                        .map(|i| gen.parameter_placeholder(*param_idx + i))
                        .collect();
                    *param_idx += arm.condition.param_count();
                    sql.push_str(" WHEN ");
                    sql.push_str(&arm.condition.to_sql(&placeholders));
                    sql.push_str(" THEN ");
                    sql.push_str(&arm.result.to_sql(gen, param_idx));
                }
                if let Some(otherwise) = otherwise {
                    sql.push_str(" ELSE ");
                    sql.push_str(&otherwise.to_sql(gen, param_idx));
                }
                sql.push_str(" END");
                sql
            }
            SqlExpr::Literal(_) => {
                let placeholder = gen.parameter_placeholder(*param_idx);
                *param_idx += 1;
                placeholder
            }
            SqlExpr::Binary { left, op, right } => format!(
                "({} {} {})",
                left.to_sql(gen, param_idx),
                op.sql_operator(),
                right.to_sql(gen, param_idx)
            ),
        }
    }

    /// Collects the bound values in the same order [`SqlExpr::to_sql`] emits
    /// their placeholders.
    pub(crate) fn collect_params(&self) -> Vec<DbValue> {
        match self {
            SqlExpr::Column(_) | SqlExpr::Aggregate { .. } => Vec::new(),
            SqlExpr::Coalesce(expr, fallback) => {
                let mut values = expr.collect_params();
                values.push(fallback.clone());
                values
            }
            SqlExpr::Case { arms, otherwise } => {
                let mut values = Vec::new();
                for arm in arms {
                    values.extend(arm.condition.values().iter().cloned());
                    values.extend(arm.result.collect_params());
                }
                if let Some(otherwise) = otherwise {
                    values.extend(otherwise.collect_params());
                }
                values
            }
            SqlExpr::Literal(value) => vec![value.clone()],
            SqlExpr::Binary { left, right, .. } => {
                let mut values = left.collect_params();
                values.extend(right.collect_params());
                values
            }
        }
    }
}

/// A logical end segment of a (possibly qualified) column name.
fn last_segment(column: &str) -> &str {
    column.rsplit('.').next().unwrap_or(column)
}

/// One SELECT-list projection: an expression plus its output alias.
///
/// `to_sql` always emits `<expr> AS <alias>`; aliases are emitted bare (like
/// the column names used by the existing query layer).
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
        let alias = last_segment(&column).to_string();
        Self {
            expr: SqlExpr::Column(column),
            alias,
        }
    }

    /// Projects `SUM(column)` with alias `sum_<column>`.
    pub fn sum(column: impl Into<String>) -> Self {
        let column = column.into();
        let alias = format!("sum_{}", last_segment(&column));
        Self {
            expr: SqlExpr::sum(column),
            alias,
        }
    }

    /// Projects `AVG(column)` with alias `avg_<column>`.
    pub fn avg(column: impl Into<String>) -> Self {
        let column = column.into();
        let alias = format!("avg_{}", last_segment(&column));
        Self {
            expr: SqlExpr::avg(column),
            alias,
        }
    }

    /// Projects `MIN(column)` with alias `min_<column>`.
    pub fn min(column: impl Into<String>) -> Self {
        let column = column.into();
        let alias = format!("min_{}", last_segment(&column));
        Self {
            expr: SqlExpr::min(column),
            alias,
        }
    }

    /// Projects `MAX(column)` with alias `max_<column>`.
    pub fn max(column: impl Into<String>) -> Self {
        let column = column.into();
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
        let alias = format!("count_{}", last_segment(&column));
        Self {
            expr: SqlExpr::count_column(column),
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
    pub fn alias(mut self, alias: impl Into<String>) -> Self {
        self.alias = alias.into();
        self
    }

    /// Compiles this projection as `<expr> AS <alias>`.
    pub(crate) fn to_sql(&self, gen: &dyn ISqlGenerator, param_idx: &mut usize) -> String {
        format!("{} AS {}", self.expr.to_sql(gen, param_idx), self.alias)
    }

    /// Collects this projection's bound values in placeholder order.
    pub(crate) fn params(&self) -> Vec<DbValue> {
        self.expr.collect_params()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::query::compile::PortablePlaceholderGenerator;

    #[test]
    fn column_alias_is_the_last_name_segment() {
        let item = SelectItem::column("o.amount");
        let mut idx = 1usize;
        let sql = item.to_sql(&PortablePlaceholderGenerator, &mut idx);
        assert_eq!(sql, "o.amount AS amount");
        assert_eq!(idx, 1, "plain columns bind nothing");
    }

    #[test]
    fn aggregate_aliases_use_the_function_prefix() {
        let mut idx = 1usize;
        let gen = PortablePlaceholderGenerator;
        assert_eq!(
            SelectItem::sum("o.amount").to_sql(&gen, &mut idx),
            "SUM(o.amount) AS sum_amount"
        );
        assert_eq!(
            SelectItem::avg("rating").to_sql(&gen, &mut idx),
            "AVG(rating) AS avg_rating"
        );
        assert_eq!(
            SelectItem::min("price").to_sql(&gen, &mut idx),
            "MIN(price) AS min_price"
        );
        assert_eq!(
            SelectItem::max("price").to_sql(&gen, &mut idx),
            "MAX(price) AS max_price"
        );
        assert_eq!(
            SelectItem::count().to_sql(&gen, &mut idx),
            "COUNT(*) AS count"
        );
        assert_eq!(
            SelectItem::count_column("b.id").to_sql(&gen, &mut idx),
            "COUNT(b.id) AS count_id"
        );
        assert_eq!(idx, 1);
    }

    #[test]
    fn coalesce_projection_binds_the_fallback_after_the_expression() {
        let item = SelectItem::sum("amount").coalesce(0i64);
        let mut idx = 1usize;
        let sql = item.to_sql(&PortablePlaceholderGenerator, &mut idx);
        assert_eq!(sql, "COALESCE(SUM(amount), ?) AS sum_amount");
        assert_eq!(item.params(), vec![DbValue::I64(0)]);
        assert_eq!(idx, 2);
    }

    #[test]
    fn literal_projection_binds_its_value() {
        let item = SelectItem::literal("TENANT", "grant_scope");
        let mut idx = 1usize;
        assert_eq!(
            item.to_sql(&PortablePlaceholderGenerator, &mut idx),
            "? AS grant_scope"
        );
        assert_eq!(item.params(), vec![DbValue::String("TENANT".into())]);
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
        let mut idx = 1usize;
        let sql = item.to_sql(&PortablePlaceholderGenerator, &mut idx);
        assert_eq!(sql, "CASE WHEN amount > ? THEN ? ELSE ? END AS size");
        assert_eq!(
            item.params(),
            vec![
                DbValue::I64(100),
                DbValue::String("big".into()),
                DbValue::String("small".into()),
            ]
        );
        assert_eq!(idx, 4);
    }

    #[test]
    fn binary_expression_compiles_with_parentheses() {
        let expr = SqlExpr::column("amount").sub(SqlExpr::literal(5i64));
        let mut idx = 1usize;
        let sql = expr.to_sql(&PortablePlaceholderGenerator, &mut idx);
        assert_eq!(sql, "(amount - ?)");
        assert_eq!(expr.collect_params(), vec![DbValue::I64(5)]);
        assert_eq!(idx, 2);
    }

    #[test]
    fn alias_override_keeps_the_expression() {
        let item = SelectItem::sum("amount").alias("total");
        let mut idx = 1usize;
        assert_eq!(
            item.to_sql(&PortablePlaceholderGenerator, &mut idx),
            "SUM(amount) AS total"
        );
    }
}
