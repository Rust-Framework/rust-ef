//! Query expression AST types.
//!
//! Boolean expression tree (`BoolExpr`), filter conditions, ordering,
//! grouping, having, includes, joins, and supporting types used across
//! the query builder and SQL compiler.

use crate::provider::DbValue;

// Use the compile-module helper for inline value collection. Declared
// here (not `super::compile::collect_bool_expr_values`) so `CompiledFilter::new`
// can pre-collect values at construction time.
use super::compile::collect_bool_expr_values;

// ---------------------------------------------------------------------------
// Query operators
// ---------------------------------------------------------------------------

/// A filter condition built from property accessors.
#[derive(Debug, Clone)]
pub struct FilterCondition {
    /// The column name this condition applies to.
    column: String,
    /// SQL operator (e.g., "=", ">", "LIKE", "IN", "BETWEEN", "IS NULL").
    operator: String,
    /// Number of bound parameters consumed by this condition.
    param_count: usize,
    /// Inline parameter values (for self-contained `BoolExpr` used outside
    /// `QueryBuilder` state, e.g. global query filters produced by
    /// `linq!(filter |b: T| ...)`). Empty for in-builder conditions where
    /// values are tracked in `QueryState::parameters`.
    pub(crate) values: Vec<DbValue>,
}

impl FilterCondition {
    pub fn new(column: impl Into<String>, operator: impl Into<String>, param_count: usize) -> Self {
        Self {
            column: column.into(),
            operator: operator.into(),
            param_count,
            values: Vec::new(),
        }
    }

    /// Creates a condition carrying its own parameter values. Used by
    /// `linq!(filter |b: T| ...)` (Form C) to produce self-contained
    /// `BoolExpr` values for global query filters.
    pub fn with_values(
        column: impl Into<String>,
        operator: impl Into<String>,
        values: Vec<DbValue>,
    ) -> Self {
        let count = values.len();
        Self {
            column: column.into(),
            operator: operator.into(),
            param_count: count,
            values,
        }
    }

    /// Returns the inline values carried by this condition (empty for
    /// in-builder conditions).
    pub fn values(&self) -> &[DbValue] {
        &self.values
    }

    /// Convert to a SQL WHERE fragment using dialect-specific placeholders.
    pub fn to_sql(&self, placeholders: &[String]) -> String {
        match self.operator.as_str() {
            "IS NULL" => format!("{} IS NULL", self.column),
            "IS NOT NULL" => format!("{} IS NOT NULL", self.column),
            "IN" => format!("{} IN ({})", self.column, placeholders.join(", ")),
            "BETWEEN" if placeholders.len() >= 2 => format!(
                "{} BETWEEN {} AND {}",
                self.column, placeholders[0], placeholders[1]
            ),
            op if self.param_count == 0 => format!("{} {}", self.column, op),
            op => format!("{} {} {}", self.column, op, placeholders[0]),
        }
    }

    pub fn param_count(&self) -> usize {
        self.param_count
    }

    pub fn column(&self) -> &str {
        &self.column
    }

    pub fn operator(&self) -> &str {
        &self.operator
    }
}

/// Wrapper for raw SQL fragments inside `BoolExpr::Raw`.
///
/// The type is `pub` (so the `BoolExpr::Raw` variant field doesn't trigger
/// `private_interfaces`), but the inner `String` field is `pub(crate)`.
/// External code can name `RawSql` but cannot construct it (field
/// inaccessible) and cannot read the SQL string — closing the raw SQL
/// injection hatch at the type level. Internal callers use
/// `BoolExpr::raw()` (`pub(crate)`).
#[derive(Debug, Clone)]
pub struct RawSql(pub(crate) String);

/// Boolean expression AST for WHERE clauses.
#[derive(Debug, Clone)]
pub enum BoolExpr {
    /// A single parameterized filter condition.
    Filter(FilterCondition),
    /// Raw SQL fragment (no parameters), e.g. global query filters.
    /// Payload is `RawSql` (`pub(crate)`) so the variant cannot be
    /// constructed by external code.
    Raw(RawSql),
    /// AND combination.
    And(Box<BoolExpr>, Box<BoolExpr>),
    /// OR combination.
    Or(Box<BoolExpr>, Box<BoolExpr>),
    /// NOT negation.
    Not(Box<BoolExpr>),
    /// EXISTS (SELECT 1 FROM related_table WHERE related.fk = outer.pk AND <predicate>)
    Exists(Box<SubquerySpec>),
    /// NOT EXISTS (...)
    NotExists(Box<SubquerySpec>),
    /// `column IN (SELECT projection FROM source_table [WHERE predicate])`
    InSubquery(Box<InSubquerySpec>),
    /// `column NOT IN (SELECT projection FROM source_table [WHERE predicate])`
    NotInSubquery(Box<InSubquerySpec>),
}

/// G5: Specification for a correlated subquery (`EXISTS` / `NOT EXISTS`).
///
/// Created by the `linq!` macro when parsing `b.posts.any(|p| p.published)`.
/// The `navigation_field` and `related_type_name` are set at macro expansion
/// time; the table/column fields are resolved at SQL generation time from
/// `EntityTypeMeta` navigation metadata.
#[derive(Debug, Clone)]
pub struct SubquerySpec {
    /// Navigation field name on the outer entity (e.g. "posts").
    pub navigation_field: String,
    /// Related entity type name (e.g. "Post").
    pub related_type_name: String,
    /// Additional predicate from the closure body (e.g. `p.published`).
    pub predicate: Option<Box<BoolExpr>>,
    /// Resolved: outer table name (e.g. "blogs").
    pub outer_table: String,
    /// Resolved: related table name (e.g. "posts").
    pub related_table: String,
    /// Resolved: FK column on the related table (e.g. "blog_id").
    pub fk_column: String,
    /// Resolved: outer entity's PK column (e.g. "id").
    pub outer_pk_column: String,
}

impl SubquerySpec {
    /// Creates an unresolved spec (table/column fields filled at SQL gen time).
    pub fn new(navigation_field: impl Into<String>, related_type_name: impl Into<String>) -> Self {
        Self {
            navigation_field: navigation_field.into(),
            related_type_name: related_type_name.into(),
            predicate: None,
            outer_table: String::new(),
            related_table: String::new(),
            fk_column: String::new(),
            outer_pk_column: String::new(),
        }
    }
}

/// v1.1: Specification for a scalar `IN (SELECT ...)` / `NOT IN (SELECT ...)`
/// subquery.
///
/// Created by the `linq!` macro when parsing
/// `b.field.in_subquery(|p: Post| p.blog_id)`. Unlike [`SubquerySpec`], this
/// variant is **not** navigation-driven — the subquery projects a single column
/// from an arbitrary table, and the outer column is compared against the
/// projected values via the `IN` operator.
#[derive(Debug, Clone)]
pub struct InSubquerySpec {
    /// The outer column being tested (e.g. `"id"` on the parent table).
    pub outer_column: String,
    /// The source table name for the inner SELECT (e.g. `"posts"`).
    pub source_table: String,
    /// The projection column selected from the inner table
    /// (e.g. `"blog_id"`).
    pub projection_column: String,
    /// Optional predicate applied inside the subquery
    /// (e.g. `WHERE published = ?`).
    pub predicate: Option<Box<BoolExpr>>,
}

impl InSubquerySpec {
    /// Creates a new IN-subquery specification.
    pub fn new(
        outer_column: impl Into<String>,
        source_table: impl Into<String>,
        projection_column: impl Into<String>,
    ) -> Self {
        Self {
            outer_column: outer_column.into(),
            source_table: source_table.into(),
            projection_column: projection_column.into(),
            predicate: None,
        }
    }
}

impl BoolExpr {
    pub fn filter(
        column: impl Into<String>,
        operator: impl Into<String>,
        param_count: usize,
    ) -> Self {
        BoolExpr::Filter(FilterCondition::new(column, operator, param_count))
    }

    pub(crate) fn raw(sql: impl Into<String>) -> Self {
        BoolExpr::Raw(RawSql(sql.into()))
    }

    pub fn and(self, other: BoolExpr) -> Self {
        BoolExpr::And(Box::new(self), Box::new(other))
    }

    pub fn or(self, other: BoolExpr) -> Self {
        BoolExpr::Or(Box::new(self), Box::new(other))
    }

    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        BoolExpr::Not(Box::new(self))
    }

    pub fn total_param_count(&self) -> usize {
        match self {
            BoolExpr::Filter(f) => f.param_count(),
            BoolExpr::Raw(_) => 0,
            BoolExpr::And(a, b) | BoolExpr::Or(a, b) => {
                a.total_param_count() + b.total_param_count()
            }
            BoolExpr::Not(inner) => inner.total_param_count(),
            BoolExpr::Exists(spec) | BoolExpr::NotExists(spec) => spec
                .predicate
                .as_ref()
                .map(|p| p.total_param_count())
                .unwrap_or(0),
            BoolExpr::InSubquery(spec) | BoolExpr::NotInSubquery(spec) => spec
                .predicate
                .as_ref()
                .map(|p| p.total_param_count())
                .unwrap_or(0),
        }
    }
}

/// An ordering specification.
#[derive(Debug, Clone)]
pub struct OrderBy {
    column: String,
    pub(crate) direction: OrderDirection,
}

#[derive(Debug, Clone, Copy)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

impl OrderBy {
    pub fn new(column: impl Into<String>, direction: OrderDirection) -> Self {
        Self {
            column: column.into(),
            direction,
        }
    }

    pub fn to_sql(&self) -> String {
        let dir = match self.direction {
            OrderDirection::Ascending => "ASC",
            OrderDirection::Descending => "DESC",
        };
        format!("{} {}", self.column, dir)
    }
}

/// A query filter with pre-collected parameter values.
///
/// Produced by `ModelBuilder::filters_by_table()`. The `expr` is retained for
/// per-query SQL compilation (which depends on the provider's dialect and the
/// current placeholder index), while `params` are collected once at
/// registration time to avoid redundant `collect_bool_expr_values` traversals
/// on every navigation/primary query.
///
/// For simple tenant filters (`tenant_id = ?`) the per-query SQL compilation
/// is a single `to_sql` call — cheap and correct for all dialects.
#[derive(Debug, Clone)]
pub struct CompiledFilter {
    /// The filter expression tree. Compiled to SQL per query using the
    /// provider's `ISqlGenerator` (placeholder syntax is dialect-specific).
    pub expr: BoolExpr,
    /// Parameter values extracted from the expression tree at registration
    /// time. Appended to the query's parameter list at apply time.
    pub params: Vec<DbValue>,
}

impl CompiledFilter {
    /// Builds a `CompiledFilter` from a `BoolExpr`, pre-collecting its
    /// inline parameter values.
    pub fn new(expr: BoolExpr) -> Self {
        let params = collect_bool_expr_values(&expr);
        Self { expr, params }
    }
}

/// An eager-load include specification.
#[derive(Debug, Clone)]
pub struct IncludePath {
    pub navigation: String,
    /// Nested ThenInclude paths (tree).
    pub nested: Vec<IncludePath>,
    /// The related table name for JOIN generation.
    pub related_table: Option<String>,
    /// The foreign key column for the JOIN condition.
    pub foreign_key_column: Option<String>,
    /// The referenced key column (typically primary key of the related table).
    pub referenced_key_column: Option<String>,
}

/// A JOIN specification for SQL generation.
#[derive(Debug, Clone)]
pub struct JoinSpec {
    /// JOIN type: "INNER", "LEFT", "RIGHT", "FULL", "CROSS"
    pub join_type: String,
    /// The table to join.
    pub table: String,
    /// The ON condition. Empty for CROSS JOIN.
    pub on_clause: String,
}

impl JoinSpec {
    pub fn to_sql(&self) -> String {
        if self.join_type == "CROSS" {
            format!("CROSS JOIN {}", self.table)
        } else {
            format!(
                "{} JOIN {} ON {}",
                self.join_type, self.table, self.on_clause
            )
        }
    }
}

/// A GROUP BY specification.
#[derive(Debug, Clone)]
pub struct GroupBy {
    pub columns: Vec<String>,
}

impl GroupBy {
    pub fn to_sql(&self) -> String {
        if self.columns.is_empty() {
            String::new()
        } else {
            format!("GROUP BY {}", self.columns.join(", "))
        }
    }
}

/// A HAVING condition.
#[derive(Debug, Clone)]
pub struct HavingCondition {
    pub expression: String,
}

impl HavingCondition {
    pub fn to_sql(&self) -> String {
        format!("HAVING {}", self.expression)
    }
}

/// Aggregate function kind for `HavingExpr`.
///
/// Used by `linq!(having ...)` (Form B) when the having clause contains
/// nested boolean expressions (`AND`/`OR`/`NOT`) or aggregate-versus-aggregate
/// comparisons (`COUNT(b.id) > SUM(b.views)`).
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

/// Comparison operator for `HavingExpr`.
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

/// AST node for `HAVING` expressions.
///
/// Supports boolean combinations (`AND`/`OR`/`NOT`) and aggregate-versus-aggregate
/// comparisons in addition to the basic `agg(col) op value` form. Generated by
/// `linq!(having ...)` (Form B) expansion and compiled to SQL by `to_sql`.
#[derive(Debug, Clone)]
pub enum HavingExpr {
    /// `agg(col) op value` — basic comparison against a literal.
    Compare {
        agg: AggKind,
        col: String,
        op: CompareOp,
        value: DbValue,
    },
    /// `expr AND expr`.
    And(Box<HavingExpr>, Box<HavingExpr>),
    /// `expr OR expr`.
    Or(Box<HavingExpr>, Box<HavingExpr>),
    /// `NOT expr`.
    Not(Box<HavingExpr>),
    /// `agg(col1) op agg(col2)` — aggregate-vs-aggregate comparison (no bound parameter).
    CompareAgg {
        left_agg: AggKind,
        left_col: String,
        op: CompareOp,
        right_agg: AggKind,
        right_col: String,
    },
}

impl HavingExpr {
    /// Recursively compiles the expression into a SQL fragment using the
    /// provider-specific placeholder syntax (`?` for SQLite/MySQL, `$N` for
    /// PostgreSQL).
    ///
    /// `param_idx` is advanced past each bound parameter in left-to-right
    /// order, matching the order produced by [`HavingExpr::collect_params`].
    /// This ensures PostgreSQL's 1-indexed `$N` placeholders stay contiguous
    /// with the WHERE clause's placeholders.
    pub fn to_sql(
        &self,
        gen: &dyn crate::provider::ISqlGenerator,
        param_idx: &mut usize,
    ) -> String {
        match self {
            Self::Compare {
                agg,
                col,
                op,
                value: _,
            } => {
                let placeholder = gen.parameter_placeholder(*param_idx);
                *param_idx += 1;
                format!(
                    "{}({}) {} {}",
                    agg.sql_name(),
                    col,
                    op.sql_name(),
                    placeholder
                )
            }
            Self::And(left, right) => format!(
                "({} AND {})",
                left.to_sql(gen, param_idx),
                right.to_sql(gen, param_idx)
            ),
            Self::Or(left, right) => format!(
                "({} OR {})",
                left.to_sql(gen, param_idx),
                right.to_sql(gen, param_idx)
            ),
            Self::Not(inner) => format!("NOT ({})", inner.to_sql(gen, param_idx)),
            Self::CompareAgg {
                left_agg,
                left_col,
                op,
                right_agg,
                right_col,
            } => format!(
                "{}({}) {} {}({})",
                left_agg.sql_name(),
                left_col,
                op.sql_name(),
                right_agg.sql_name(),
                right_col
            ),
        }
    }

    /// Collects bound parameter values in the same left-to-right order that
    /// [`HavingExpr::to_sql`] emits placeholders. Used to populate the query
    /// parameter vector at registration time so that `compile_sql` returns
    /// params matching the placeholder order.
    pub fn collect_params(&self) -> Vec<DbValue> {
        match self {
            Self::Compare { value, .. } => vec![value.clone()],
            Self::And(left, right) | Self::Or(left, right) => {
                let mut v = left.collect_params();
                v.extend(right.collect_params());
                v
            }
            Self::Not(inner) => inner.collect_params(),
            Self::CompareAgg { .. } => Vec::new(),
        }
    }
}
