//! Query builder ??LINQ-style chainable query API.
//!
//! Accumulates filter conditions, orderings, pagination, includes, and
//! projection metadata through a fluent interface. Terminal methods
//! (`to_list`, `first`, `count`, etc.) produce real SQL that can be
//! executed against a database provider.

use crate::entity::{
    IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, ILazyInit, INavigationSetter,
};
use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, DbValueConvertError, IDatabaseProvider};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

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

/// Boolean expression AST for WHERE clauses.
#[derive(Debug, Clone)]
pub enum BoolExpr {
    /// A single parameterized filter condition.
    Filter(FilterCondition),
    /// Raw SQL fragment (no parameters), e.g. global query filters.
    Raw(String),
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

    pub fn raw(sql: impl Into<String>) -> Self {
        BoolExpr::Raw(sql.into())
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
    direction: OrderDirection,
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
    /// JOIN type: "INNER", "LEFT", "RIGHT"
    pub join_type: String,
    /// The table to join.
    pub table: String,
    /// The ON condition.
    pub on_clause: String,
}

impl JoinSpec {
    pub fn to_sql(&self) -> String {
        format!(
            "{} JOIN {} ON {}",
            self.join_type, self.table, self.on_clause
        )
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

// ---------------------------------------------------------------------------
// Window functions & CTE (v1.1)
// ---------------------------------------------------------------------------

/// Kinds of window function supported by `linq!(window ...)`.
///
/// Ranking functions (`RowNumber`, `Rank`, `DenseRank`) take no column
/// argument; aggregate functions (`Sum`, `Count`, ...) and offset functions
/// (`Lag`, `Lead`) take a single column argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowFuncKind {
    RowNumber,
    Rank,
    DenseRank,
    Lag,
    Lead,
    Sum,
    Count,
    Avg,
    Min,
    Max,
}

impl WindowFuncKind {
    /// Returns the SQL keyword for this window function.
    pub fn sql_name(&self) -> &'static str {
        match self {
            WindowFuncKind::RowNumber => "ROW_NUMBER",
            WindowFuncKind::Rank => "RANK",
            WindowFuncKind::DenseRank => "DENSE_RANK",
            WindowFuncKind::Lag => "LAG",
            WindowFuncKind::Lead => "LEAD",
            WindowFuncKind::Sum => "SUM",
            WindowFuncKind::Count => "COUNT",
            WindowFuncKind::Avg => "AVG",
            WindowFuncKind::Min => "MIN",
            WindowFuncKind::Max => "MAX",
        }
    }

    /// Parses a window function name (case-insensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "ROW_NUMBER" => Some(WindowFuncKind::RowNumber),
            "RANK" => Some(WindowFuncKind::Rank),
            "DENSE_RANK" => Some(WindowFuncKind::DenseRank),
            "LAG" => Some(WindowFuncKind::Lag),
            "LEAD" => Some(WindowFuncKind::Lead),
            "SUM" => Some(WindowFuncKind::Sum),
            "COUNT" => Some(WindowFuncKind::Count),
            "AVG" => Some(WindowFuncKind::Avg),
            "MIN" => Some(WindowFuncKind::Min),
            "MAX" => Some(WindowFuncKind::Max),
            _ => None,
        }
    }

    /// Whether this function takes a column argument.
    pub fn takes_column(&self) -> bool {
        !matches!(
            self,
            WindowFuncKind::RowNumber | WindowFuncKind::Rank | WindowFuncKind::DenseRank
        )
    }
}

/// Specification for a window function projection.
///
/// Mirrors the design of [`HavingExpr`]: a structured AST node stored in
/// [`QueryState`] and compiled to SQL at generation time so that dialect-
/// specific identifier quoting is applied consistently.
#[derive(Debug, Clone)]
pub struct WindowSpec {
    /// The window function to apply.
    pub func: WindowFuncKind,
    /// The column argument (required for aggregate/offset functions,
    /// ignored for ranking functions).
    pub column: Option<String>,
    /// PARTITION BY columns (empty for no partitioning).
    pub partition_by: Vec<String>,
    /// ORDER BY within the window frame.
    pub order_by: Vec<(String, OrderDirection)>,
    /// The output column alias (emitted as `AS <alias>`).
    pub alias: String,
}

impl WindowSpec {
    /// Compiles this window function into a SELECT-list projection fragment
    /// using the provider's identifier quoting.
    ///
    /// Ranking functions emit `FUNC() OVER (...)`; aggregate/offset functions
    /// emit `FUNC(col) OVER (...)`. The alias is always appended.
    pub fn to_sql(&self, gen: &dyn crate::provider::ISqlGenerator) -> String {
        let func_name = self.func.sql_name();
        let arg = if self.func.takes_column() {
            let col = self.column.as_deref().unwrap_or("*");
            gen.quote_identifier(col)
        } else {
            String::new()
        };
        let call = if self.func.takes_column() {
            format!("{}({})", func_name, arg)
        } else {
            format!("{}()", func_name)
        };

        let mut over = String::new();
        if !self.partition_by.is_empty() {
            let parts: Vec<String> = self
                .partition_by
                .iter()
                .map(|c| gen.quote_identifier(c))
                .collect();
            over.push_str(&format!("PARTITION BY {}", parts.join(", ")));
        }
        if !self.order_by.is_empty() {
            if !over.is_empty() {
                over.push(' ');
            }
            let parts: Vec<String> = self
                .order_by
                .iter()
                .map(|(c, d)| {
                    let quoted = gen.quote_identifier(c);
                    let dir = match d {
                        OrderDirection::Ascending => "ASC",
                        OrderDirection::Descending => "DESC",
                    };
                    format!("{} {}", quoted, dir)
                })
                .collect();
            over.push_str(&format!("ORDER BY {}", parts.join(", ")));
        }
        let alias = gen.quote_identifier(&self.alias);
        format!("{} OVER ({}) AS {}", call, over, alias)
    }
}

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
}

// ---------------------------------------------------------------------------
// Query state
// ---------------------------------------------------------------------------

/// Accumulated intent for a single query.
#[derive(Debug, Clone)]
pub struct QueryState {
    /// FROM table / subquery segment.
    pub from: String,
    /// WHERE clause conditions.
    pub filters: Vec<FilterCondition>,
    /// JOIN specifications.
    pub joins: Vec<JoinSpec>,
    /// GROUP BY columns.
    pub group_bys: Vec<String>,
    /// HAVING conditions stored as AST nodes so they can be compiled with the
    /// provider-specific placeholder syntax (`?` vs `$N`) at SQL generation
    /// time, rather than being pre-compiled to a fixed placeholder.
    pub havings: Vec<HavingExpr>,
    /// ORDER BY clauses.
    pub orderings: Vec<OrderBy>,
    /// OFFSET (Skip).
    pub offset: Option<usize>,
    /// LIMIT (Take).
    pub limit: Option<usize>,
    /// Include navigation paths.
    pub includes: Vec<IncludePath>,
    /// Whether this is a projection (SELECT col1, col2 instead of SELECT *).
    pub projected_columns: Option<Vec<String>>,
    /// Whether this is a COUNT query.
    pub is_count: bool,
    /// Whether this is an EXISTS sub-query.
    pub is_exists: bool,
    /// Aggregate function to apply: "SUM", "AVG", "MIN", "MAX", "COUNT"
    pub aggregate: Option<String>,
    /// The column to aggregate.
    pub aggregate_column: Option<String>,
    /// Collected parameter values in order of appearance.
    pub parameters: Vec<DbValue>,
    /// Boolean WHERE expression (preferred over `filters`).
    pub where_expr: Option<BoolExpr>,
    /// Whether to emit `SELECT DISTINCT`.
    pub distinct: bool,
    /// Window function projections (v1.1). Emitted in the SELECT list as
    /// `func(col) OVER (PARTITION BY ... ORDER BY ...) AS alias`.
    pub windows: Vec<WindowSpec>,
    /// CTE definitions (v1.1). Emitted as `WITH name AS (...)` prefix
    /// before the SELECT. CTE parameters are prepended to the query's
    /// parameter list at execution time.
    pub ctes: Vec<CteSpec>,
}

impl QueryState {
    pub fn new(from: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            filters: Vec::new(),
            joins: Vec::new(),
            group_bys: Vec::new(),
            havings: Vec::new(),
            orderings: Vec::new(),
            offset: None,
            limit: None,
            includes: Vec::new(),
            projected_columns: None,
            is_count: false,
            is_exists: false,
            aggregate: None,
            aggregate_column: None,
            parameters: Vec::new(),
            where_expr: None,
            distinct: false,
            windows: Vec::new(),
            ctes: Vec::new(),
        }
    }

    fn append_bool_expr(&mut self, expr: BoolExpr) {
        self.where_expr = Some(match self.where_expr.take() {
            None => expr,
            Some(existing) => BoolExpr::And(Box::new(existing), Box::new(expr)),
        });
    }

    fn append_filter(&mut self, condition: FilterCondition) {
        self.filters.push(condition.clone());
        self.append_bool_expr(BoolExpr::Filter(condition));
    }

    /// Compile the state into a SQL string using the provider's placeholder style.
    pub fn to_sql_with(&self, gen: &dyn crate::provider::ISqlGenerator) -> String {
        // CTE parameter count — the main query's PostgreSQL `$N` placeholders
        // must continue from this offset to stay contiguous with CTE params.
        let cte_param_count: usize = self.ctes.iter().map(|c| c.params.len()).sum();

        let distinct_kw = if self.distinct { "DISTINCT " } else { "" };
        let select = if self.is_count {
            if self.distinct {
                "SELECT COUNT(DISTINCT *)".to_string()
            } else {
                "SELECT COUNT(*)".to_string()
            }
        } else if self.is_exists {
            "SELECT 1".to_string()
        } else if let Some(ref agg) = self.aggregate {
            let col = self.aggregate_column.as_deref().unwrap_or("*");
            if self.distinct {
                format!("SELECT {}(DISTINCT {})", agg, col)
            } else {
                format!("SELECT {}({})", agg, col)
            }
        } else if let Some(ref cols) = self.projected_columns {
            let mut parts: Vec<String> = cols.to_vec();
            // Window function projections are appended to explicit column lists.
            for w in &self.windows {
                parts.push(w.to_sql(gen));
            }
            format!("SELECT {}{}", distinct_kw, parts.join(", "))
        } else {
            // Default SELECT * — append window projections if present.
            if self.windows.is_empty() {
                format!("SELECT {}*", distinct_kw)
            } else {
                let mut parts: Vec<String> = vec![format!("{}*", distinct_kw)];
                for w in &self.windows {
                    parts.push(w.to_sql(gen));
                }
                format!("SELECT {}", parts.join(", "))
            }
        };

        let mut sql = format!("{} FROM {}", select, self.from);

        // JOINs
        for join in &self.joins {
            sql.push_str(&format!(" {}", join.to_sql()));
        }

        // Parameter index is shared across WHERE and HAVING so that
        // PostgreSQL's 1-indexed `$N` placeholders remain contiguous and
        // correctly ordered across both clauses. CTE parameters (if any)
        // occupy the leading slots, so the main query starts after them.
        let mut param_idx = 1usize + cte_param_count;

        // WHERE
        if let Some(ref expr) = self.where_expr {
            sql.push_str(&format!(
                " WHERE {}",
                compile_bool_expr(expr, gen, &mut param_idx)
            ));
        } else if !self.filters.is_empty() {
            sql.push_str(&format!(
                " WHERE {}",
                build_where_clauses(&self.filters, gen)
            ));
            // Advance `param_idx` past the legacy `filters` path so that
            // HAVING placeholders (PostgreSQL `$N`) continue from the
            // correct index. `build_where_clauses` always starts at index 1.
            param_idx += self.filters.iter().map(|f| f.param_count()).sum::<usize>();
        }

        // GROUP BY
        if !self.group_bys.is_empty() {
            sql.push_str(&format!(" GROUP BY {}", self.group_bys.join(", ")));
        }

        // HAVING — compile each `HavingExpr` AST node with the provider's
        // placeholder syntax, continuing the shared `param_idx` so PostgreSQL
        // `$N` indices stay contiguous with the WHERE clause.
        if !self.havings.is_empty() {
            let compiled: Vec<String> = self
                .havings
                .iter()
                .map(|h| h.to_sql(gen, &mut param_idx))
                .collect();
            sql.push_str(&format!(" HAVING {}", compiled.join(" AND ")));
        }

        // ORDER BY
        if !self.orderings.is_empty() {
            let ords: Vec<String> = self.orderings.iter().map(|o| o.to_sql()).collect();
            sql.push_str(&format!(" ORDER BY {}", ords.join(", ")));
        }

        // LIMIT / OFFSET — delegated to the dialect-specific generator so
        // that PostgreSQL emits `OFFSET x LIMIT y`, MySQL handles the
        // offset-only case via `LIMIT 18446744073709551615 OFFSET y`, and
        // SQLite/MySQL use `LIMIT x OFFSET y`.
        let pagination = gen.pagination(self.offset, self.limit);
        if !pagination.is_empty() {
            sql.push(' ');
            sql.push_str(&pagination);
        }

        // CTE prefix — emitted as `WITH name AS (body), ...` before the SELECT.
        //
        // Two modes:
        // - **Raw mode** (`sql` non-empty): body is the pre-compiled SQL,
        //   emitted verbatim with `?` placeholders.
        // - **Typed mode** (`table` non-empty): body is compiled from
        //   `where_expr` at this point using the provider's placeholder
        //   syntax. Parameter values were already extracted into `params` at
        //   construction time (see `with_cte_typed`), so `param_idx` for the
        //   main query starts at `1 + cte_param_count` (computed above).
        //
        // `running_idx` accumulates across all CTEs so that PostgreSQL's
        // 1-indexed `$N` placeholders stay contiguous across multiple typed
        // CTEs and align with each CTE's slot in `all_params()`. Raw-mode
        // CTEs advance `running_idx` by their param count for consistency,
        // even though their `?` placeholders don't use the index.
        if !self.ctes.is_empty() {
            let mut running_idx = 1usize;
            let mut cte_parts: Vec<String> = Vec::with_capacity(self.ctes.len());
            for c in &self.ctes {
                let body = if !c.table.is_empty() {
                    // Typed mode: compile WHERE expression starting at the
                    // running index. `cte_idx` advances as placeholders are
                    // emitted; we then propagate it back to `running_idx`.
                    let mut cte_idx = running_idx;
                    let table = gen.quote_identifier(&c.table);
                    let body = match &c.where_expr {
                        Some(expr) => {
                            let where_sql = compile_bool_expr(expr, gen, &mut cte_idx);
                            format!("SELECT * FROM {} WHERE {}", table, where_sql)
                        }
                        None => format!("SELECT * FROM {}", table),
                    };
                    running_idx = cte_idx;
                    body
                } else {
                    // Raw mode: pre-compiled SQL with `?` placeholders. The
                    // index isn't consumed but advance for consistency with
                    // `all_params()` ordering.
                    running_idx = running_idx.saturating_add(c.params.len());
                    c.sql.clone()
                };

                let part = if c.columns.is_empty() {
                    format!("{} AS ({})", c.name, body)
                } else {
                    let cols = c
                        .columns
                        .iter()
                        .map(|col| gen.quote_identifier(col))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{} ({}) AS ({})", c.name, cols, body)
                };
                cte_parts.push(part);
            }
            sql = format!("WITH {} {}", cte_parts.join(", "), sql);
        }

        sql
    }

    /// Returns all parameter values for execution: CTE parameters first
    /// (in declaration order), followed by WHERE/HAVING parameters.
    ///
    /// This ordering matches the placeholder order in the generated SQL,
    /// where CTE bodies appear before the main SELECT.
    pub fn all_params(&self) -> Vec<DbValue> {
        let mut params = Vec::new();
        for cte in &self.ctes {
            params.extend(cte.params.clone());
        }
        params.extend(self.parameters.clone());
        params
    }

    /// Compile SQL with `?` placeholders (SQLite/MySQL style).
    pub fn to_sql(&self) -> String {
        self.to_sql_with(&PortablePlaceholderGenerator)
    }

    /// Returns the accumulated parameters.
    pub fn params(&self) -> &[DbValue] {
        &self.parameters
    }
}

/// Fallback generator for SQL-only queries without an attached provider.
struct PortablePlaceholderGenerator;

impl crate::provider::ISqlGenerator for PortablePlaceholderGenerator {
    fn select(&self, _: &str, _: &[&str]) -> String {
        String::new()
    }
    fn insert(&self, _: &str, _: &[&str], _: bool) -> String {
        String::new()
    }
    fn update(&self, _: &str, _: &[&str], _: &str) -> String {
        String::new()
    }
    fn delete(&self, _: &str, _: &str) -> String {
        String::new()
    }
    fn create_table(&self, _: &str, _: &[(String, String)]) -> String {
        String::new()
    }
    fn drop_table(&self, _: &str) -> String {
        String::new()
    }
    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String {
        // Portable fallback: emit the most widely-supported form
        // `LIMIT x OFFSET y`. SQLite and MySQL accept this verbatim;
        // PostgreSQL also accepts this clause order (though it prefers
        // `OFFSET x LIMIT y`). Offset-only without a LIMIT is only
        // supported by PostgreSQL and SQLite, so we emit `OFFSET y` and
        // rely on the caller to attach a real provider when targeting
        // MySQL (whose offset-only case requires a sentinel LIMIT).
        match (skip, take) {
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            (Some(s), None) => format!("OFFSET {}", s),
            (None, None) => String::new(),
        }
    }
    fn parameter_placeholder(&self, _: usize) -> String {
        "?".to_string()
    }
    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }
    fn auto_increment_syntax(&self) -> &'static str {
        "AUTOINCREMENT"
    }
}

// ---------------------------------------------------------------------------
// QueryBuilder
// ---------------------------------------------------------------------------

/// Converts the first cell of the first row of an aggregation query result
/// into the target type `V`. Returns `None` when no rows, no cells, or a SQL
/// NULL was returned (e.g. `MIN`/`MAX` over an empty input set). The driver
/// returns `String` cells, so the value is wrapped in `DbValue::String`
/// before `TryFrom` conversion.
fn convert_aggregate_cell<V>(rows: Vec<Vec<String>>) -> EFResult<Option<V>>
where
    V: TryFrom<DbValue, Error = DbValueConvertError>,
{
    match rows.first().and_then(|r| r.first()) {
        Some(s) if s.eq_ignore_ascii_case("NULL") => Ok(None),
        Some(s) => {
            let db_val = DbValue::String(s.clone());
            V::try_from(db_val)
                .map(Some)
                .map_err(crate::error::EFError::from)
        }
        None => Ok(None),
    }
}

/// A chainable query builder for entity type `T`.
///
/// Corresponds to EFCore's `IQueryable<T>`.
///
/// `Clone` is derived so that builders can be forked for compositional reuse
/// (e.g. applying additional filters on a base query without losing the
/// original). Note that `single`/`single_or_default` still use the `take(2)`
/// approach rather than `clone().count()` to avoid a double round-trip.
#[derive(Clone)]
pub struct QueryBuilder<T: IEntityType> {
    state: QueryState,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    filter_map: Option<Arc<HashMap<String, CompiledFilter>>>,
    lazy_loading_enabled: bool,
    _phantom: PhantomData<T>,
}

impl<T: IEntityType> QueryBuilder<T> {
    /// Creates a new QueryBuilder for a given table (without provider ??SQL-only).
    pub fn new(table_name: impl Into<String>) -> Self {
        Self {
            state: QueryState::new(table_name),
            provider: None,
            filter_map: None,
            lazy_loading_enabled: false,
            _phantom: PhantomData,
        }
    }

    /// Creates a new QueryBuilder for a given table with a provider for execution.
    pub fn with_provider(
        table_name: impl Into<String>,
        provider: Arc<dyn IDatabaseProvider>,
    ) -> Self {
        Self {
            state: QueryState::new(table_name),
            provider: Some(provider),
            filter_map: None,
            lazy_loading_enabled: false,
            _phantom: PhantomData,
        }
    }

    /// Attaches a global filter map (table_name → BoolExpr) for NavigationLoader.
    pub(crate) fn with_filter_map(
        mut self,
        map: Option<Arc<HashMap<String, CompiledFilter>>>,
    ) -> Self {
        self.filter_map = map;
        self
    }

    /// Sets whether lazy loading is enabled for materialized entities.
    pub(crate) fn with_lazy_loading(mut self, enabled: bool) -> Self {
        self.lazy_loading_enabled = enabled;
        self
    }

    /// Returns a reference to the accumulated query state.
    pub fn state(&self) -> &QueryState {
        &self.state
    }

    /// Applies a compile-time LINQ expression tree from `linq!(?)`.
    pub fn filter(self, f: impl FnOnce(Self) -> Self) -> Self {
        f(self)
    }

    // -------------------------------------------------------------------
    // `linq!` expansion targets (`#[doc(hidden)]`)
    // -------------------------------------------------------------------

    #[doc(hidden)]
    pub fn filter_column(
        mut self,
        column: &str,
        operator: &str,
        value: impl Into<DbValue>,
    ) -> Self {
        let db_val = value.into();
        self.state.parameters.push(db_val);
        self.state
            .append_filter(FilterCondition::new(column, operator, 1));
        self
    }

    #[doc(hidden)]
    pub fn filter_not(mut self, column: &str, operator: &str, value: impl Into<DbValue>) -> Self {
        let db_val = value.into();
        self.state.parameters.push(db_val);
        self.state
            .append_bool_expr(BoolExpr::Not(Box::new(BoolExpr::Filter(
                FilterCondition::new(column, operator, 1),
            ))));
        self
    }

    #[doc(hidden)]
    pub fn filter_in(mut self, column: &str, values: Vec<DbValue>) -> Self {
        let count = values.len();
        for v in values {
            self.state.parameters.push(v);
        }
        self.state
            .append_filter(FilterCondition::new(column, "IN", count));
        self
    }

    #[doc(hidden)]
    pub fn filter_not_in(mut self, column: &str, values: Vec<DbValue>) -> Self {
        let count = values.len();
        for v in values {
            self.state.parameters.push(v);
        }
        self.state
            .append_bool_expr(BoolExpr::Not(Box::new(BoolExpr::Filter(
                FilterCondition::new(column, "IN", count),
            ))));
        self
    }

    #[doc(hidden)]
    pub fn filter_is_null(mut self, column: &str) -> Self {
        self.state
            .append_filter(FilterCondition::new(column, "IS NULL", 0));
        self
    }

    #[doc(hidden)]
    pub fn filter_is_not_null(mut self, column: &str) -> Self {
        self.state
            .append_filter(FilterCondition::new(column, "IS NOT NULL", 0));
        self
    }

    #[doc(hidden)]
    pub fn filter_between(
        mut self,
        column: &str,
        low: impl Into<DbValue>,
        high: impl Into<DbValue>,
    ) -> Self {
        let lo: DbValue = low.into();
        let hi: DbValue = high.into();
        self.state.parameters.push(lo);
        self.state.parameters.push(hi);
        self.state
            .append_filter(FilterCondition::new(column, "BETWEEN", 2));
        self
    }

    #[doc(hidden)]
    pub fn filter_like(self, column: &str, pattern: impl Into<DbValue>) -> Self {
        self.filter_column(column, "LIKE", pattern)
    }

    #[doc(hidden)]
    pub fn filter_not_like(self, column: &str, pattern: impl Into<DbValue>) -> Self {
        self.filter_not(column, "LIKE", pattern)
    }

    #[doc(hidden)]
    pub fn order_by_column(mut self, column: &str) -> Self {
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Ascending));
        self
    }

    #[doc(hidden)]
    pub fn order_by_desc_column(mut self, column: &str) -> Self {
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Descending));
        self
    }

    /// Applies a global query filter `BoolExpr` (produced by `linq!(filter |b: T| ...)`).
    /// Inline values carried by the expression are collected and appended to
    /// the query parameters in the correct position.
    pub(crate) fn apply_query_filter(mut self, filter: BoolExpr) -> Self {
        let values = collect_bool_expr_values(&filter);
        self.state.parameters.extend(values);
        self.state.append_bool_expr(filter);
        self
    }

    /// Marks this query as `SELECT DISTINCT`.
    pub fn distinct(mut self) -> Self {
        self.state.distinct = true;
        self
    }

    #[doc(hidden)]
    pub fn or_where(mut self, f: impl FnOnce(QueryBuilder<T>) -> QueryBuilder<T>) -> Self {
        let sub = f(QueryBuilder {
            state: QueryState::new(&self.state.from),
            provider: self.provider.clone(),
            filter_map: None,
            lazy_loading_enabled: false,
            _phantom: PhantomData,
        });
        let right = sub.state.where_expr.or_else(|| {
            if sub.state.filters.is_empty() {
                None
            } else {
                Some(filters_to_and_expr(&sub.state.filters))
            }
        });
        if let Some(right_expr) = right {
            self.state.where_expr = Some(match self.state.where_expr.take() {
                None => right_expr,
                Some(left) => BoolExpr::Or(Box::new(left), Box::new(right_expr)),
            });
            self.state.parameters.extend(sub.state.parameters);
            self.state.filters.extend(sub.state.filters);
        }
        self
    }

    /// G5: Adds an `EXISTS` (or `NOT EXISTS`) correlated subquery condition.
    ///
    /// `#[doc(hidden)]` — called by `linq!` expansion of
    /// `b.posts.any(|p| p.published)` / `b.posts.none(...)` / `b.posts.all(...)`.
    ///
    /// The `nav_field` and `related_type` arguments are the `&'static str`
    /// constants emitted by `#[derive(EntityType)]` (`FIELD_<NAME>` and
    /// `NAV_RELATED_<NAME>`). The table/column fields of the `SubquerySpec`
    /// are resolved later at SQL generation time via `resolve_subqueries`.
    pub fn where_exists_internal(
        mut self,
        nav_field: &'static str,
        related_type: &'static str,
        predicate: Option<BoolExpr>,
        negated: bool,
    ) -> Self {
        let mut spec = SubquerySpec::new(nav_field, related_type);
        if let Some(pred) = predicate {
            // Collect self-contained values from the predicate (e.g. the
            // `DbValue::Bool(true)` from `p.published`) and append them to
            // the query parameters. The `?` placeholders generated by
            // `compile_bool_expr` will reference them in order.
            let values = collect_bool_expr_values(&pred);
            self.state.parameters.extend(values);
            spec.predicate = Some(Box::new(pred));
        }
        let expr = if negated {
            BoolExpr::NotExists(Box::new(spec))
        } else {
            BoolExpr::Exists(Box::new(spec))
        };
        self.state.append_bool_expr(expr);
        self
    }

    /// v1.1: Adds an `IN (SELECT ...)` (or `NOT IN (SELECT ...)`) subquery
    /// condition.
    ///
    /// `#[doc(hidden)]` — called by `linq!` expansion of
    /// `b.field.in_subquery(|p: Post| p.blog_id)`.
    ///
    /// Unlike `where_exists_internal`, the `InSubquerySpec` is fully
    /// specified at construction time (no navigation resolution needed).
    /// The `source_table` and `projection_column` are `&'static str`
    /// constants emitted by `#[derive(EntityType)]` (`TABLE` and
    /// `COLUMN_<NAME>`).
    pub fn where_in_subquery_internal(
        mut self,
        outer_column: &'static str,
        source_table: &'static str,
        projection_column: &'static str,
        predicate: Option<BoolExpr>,
        negated: bool,
    ) -> Self {
        let mut spec = InSubquerySpec::new(outer_column, source_table, projection_column);
        if let Some(pred) = predicate {
            let values = collect_bool_expr_values(&pred);
            self.state.parameters.extend(values);
            spec.predicate = Some(Box::new(pred));
        }
        let expr = if negated {
            BoolExpr::NotInSubquery(Box::new(spec))
        } else {
            BoolExpr::InSubquery(Box::new(spec))
        };
        self.state.append_bool_expr(expr);
        self
    }

    // -------------------------------------------------------------------
    // Chainable methods (each returns Self with accumulated state)
    // -------------------------------------------------------------------

    /// Finds an entity by its single primary key. Uses the entity's PK
    /// metadata — no longer hardcodes `"id"`.
    pub async fn find(self, id: impl Into<DbValue>) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let meta = T::entity_meta();
        let pk_col = meta
            .primary_keys
            .first()
            .map(|s| s.as_ref())
            .or_else(|| {
                meta.properties
                    .iter()
                    .find(|p| p.is_primary_key)
                    .map(|p| p.column_name.as_ref())
            })
            .ok_or_else(|| {
                crate::error::EFError::Query(format!(
                    "entity {} has no primary key defined",
                    std::any::type_name::<T>()
                ))
            })?;
        let col_const = pk_col.to_string();
        self.filter_column(&col_const, "=", id)
            .first_or_default()
            .await
    }

    /// Finds an entity by composite primary key. Keys are column-name
    /// constants paired with values, e.g. `&[(BlogTag::COLUMN_BLOG_ID, DbValue::I32(1))]`.
    pub async fn find_by_key(mut self, keys: &[(&str, DbValue)]) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        for (col, val) in keys {
            self = self.filter_column(col, "=", val.clone());
        }
        self.first_or_default().await
    }

    /// Checks if an entity with the given single primary key exists.
    ///
    /// Uses `SELECT 1 ... LIMIT 1` — cheaper than `find(id).await?.is_some()`
    /// which materializes the full row. Reads the PK column from entity
    /// metadata, mirroring [`find`](Self::find).
    pub async fn exists_by_id(self, id: impl Into<DbValue>) -> EFResult<bool> {
        let meta = T::entity_meta();
        let pk_col = meta
            .primary_keys
            .first()
            .map(|s| s.as_ref())
            .or_else(|| {
                meta.properties
                    .iter()
                    .find(|p| p.is_primary_key)
                    .map(|p| p.column_name.as_ref())
            })
            .ok_or_else(|| {
                crate::error::EFError::Query(format!(
                    "entity {} has no primary key defined",
                    std::any::type_name::<T>()
                ))
            })?;
        let col_const = pk_col.to_string();
        self.filter_column(&col_const, "=", id).any().await
    }

    /// Checks if an entity with the given composite key exists.
    ///
    /// Uses `SELECT 1 ... LIMIT 1` — cheaper than `find_by_key(keys).is_some()`.
    pub async fn exists_by_key(mut self, keys: &[(&str, DbValue)]) -> EFResult<bool> {
        for (col, val) in keys {
            self = self.filter_column(col, "=", val.clone());
        }
        self.any().await
    }

    /// Skips the specified number of rows.
    pub fn skip(mut self, count: usize) -> Self {
        self.state.offset = Some(count);
        self
    }

    /// Takes the specified number of rows.
    pub fn take(mut self, count: usize) -> Self {
        self.state.limit = Some(count);
        self
    }

    /// Eagerly loads a named navigation (resolves FK/table from entity metadata).
    ///
    /// `#[doc(hidden)]` — called by `linq!(include b.posts)` expansion. Users
    /// should use the `linq!` macro instead of calling this directly.
    #[doc(hidden)]
    pub fn include_internal(mut self, navigation: &'static str) -> Self {
        let meta = T::entity_meta();
        let nav_meta = meta.find_navigation(navigation);
        let (related_table, fk_col, ref_col) = nav_meta
            .map(|n| {
                (
                    n.related_table.as_ref().map(|s| s.to_string()),
                    n.fk_column.as_ref().map(|s| s.to_string()),
                    n.referenced_key_column.as_ref().map(|s| s.to_string()),
                )
            })
            .unwrap_or((None, None, None));

        self.state.includes.push(IncludePath {
            navigation: navigation.to_string(),
            nested: Vec::new(),
            related_table,
            foreign_key_column: fk_col,
            referenced_key_column: ref_col,
        });
        self
    }

    /// Eagerly loads a nested navigation on the last `include_internal` path.
    ///
    /// `#[doc(hidden)]` — called by `linq!(include b.posts then b.comments)`
    /// expansion. The nested navigation field name is a string literal because
    /// the entity type transition is runtime knowledge (resolved via metadata).
    #[doc(hidden)]
    pub fn then_include_internal(mut self, navigation: &'static str) -> Self {
        if let Some(last) = self.state.includes.last_mut() {
            let parent_meta = T::entity_meta();
            if let Some(parent_nav) = parent_meta.find_navigation(&last.navigation) {
                if let Some(meta_fn) = parent_nav.related_entity_meta {
                    let related_meta = meta_fn();
                    if let Some(nav_meta) = related_meta.find_navigation(navigation) {
                        last.nested.push(IncludePath {
                            navigation: navigation.to_string(),
                            nested: Vec::new(),
                            related_table: nav_meta.related_table.as_ref().map(|s| s.to_string()),
                            foreign_key_column: nav_meta.fk_column.as_ref().map(|s| s.to_string()),
                            referenced_key_column: nav_meta
                                .referenced_key_column
                                .as_ref()
                                .map(|s| s.to_string()),
                        });
                    }
                }
            }
        }
        self
    }

    /// Adds an INNER JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(inner_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn inner_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "INNER".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a LEFT JOIN.
    ///
    /// `#[doc(hidden)]` — called by `linq!(left_join |a: T1, b: T2| a.col == b.col)`
    /// expansion.
    #[doc(hidden)]
    pub fn left_join_internal(
        mut self,
        table: &'static str,
        left_column: &'static str,
        right_column: &'static str,
    ) -> Self {
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, left_column, table, right_column
        );
        self.state.joins.push(JoinSpec {
            join_type: "LEFT".to_string(),
            table: table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds a GROUP BY clause.
    ///
    /// `#[doc(hidden)]` — called by `linq!(group_by (b.cat, b.author))` expansion.
    #[doc(hidden)]
    pub fn group_by_internal(mut self, columns: &'static [&'static str]) -> Self {
        self.state.group_bys = columns.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Adds a HAVING condition.
    ///
    /// `#[doc(hidden)]` — called by `linq!(having count(b.id) > 1)` expansion.
    /// Constructs `agg(column) op ?` with the value pushed to parameters.
    #[doc(hidden)]
    pub fn having_internal(
        mut self,
        agg: &str,
        column: &str,
        op: &str,
        value: impl Into<DbValue>,
    ) -> Self {
        let agg_kind = AggKind::from_name(agg)
            .unwrap_or_else(|| panic!("invalid aggregate name in having_internal: {agg}"));
        let cmp_op = CompareOp::from_symbol(op)
            .unwrap_or_else(|| panic!("invalid operator in having_internal: {op}"));
        let db_val = value.into();
        self.state.parameters.push(db_val.clone());
        self.state.havings.push(HavingExpr::Compare {
            agg: agg_kind,
            col: column.to_string(),
            op: cmp_op,
            value: db_val,
        });
        self
    }

    /// Adds a HAVING condition from a `HavingExpr` AST.
    ///
    /// `#[doc(hidden)]` — called by `linq!(having <expr>)` expansion when the
    /// having clause contains boolean combinations (`AND`/`OR`/`NOT`) or
    /// aggregate-versus-aggregate comparisons. The expression is stored as an
    /// AST node and compiled to SQL at `to_sql_with` time using the provider's
    /// placeholder syntax; bound parameters are collected via
    /// [`HavingExpr::collect_params`] and pushed to `state.parameters`.
    #[doc(hidden)]
    pub fn having_expr_internal(mut self, expr: HavingExpr) -> Self {
        self.state.parameters.extend(expr.collect_params());
        self.state.havings.push(expr);
        self
    }

    // -------------------------------------------------------------------
    // Window functions & CTE (v1.1)
    // -------------------------------------------------------------------

    /// Adds a window function projection to the SELECT list.
    ///
    /// `#[doc(hidden)]` — called by `linq!(window ...)` expansion.
    ///
    /// - `func`: window function name (e.g. `"row_number"`, `"sum"`, `"lag"`).
    /// - `column`: the column argument (`None` for ranking functions).
    /// - `partition_by`: PARTITION BY columns.
    /// - `order_by`: ORDER BY columns as `(column, descending)` pairs.
    /// - `alias`: the output column alias.
    #[doc(hidden)]
    pub fn window_internal(
        mut self,
        func: &str,
        column: Option<&str>,
        partition_by: &'static [&'static str],
        order_by: &'static [(&'static str, bool)],
        alias: &str,
    ) -> Self {
        let kind = WindowFuncKind::from_name(func)
            .unwrap_or_else(|| panic!("invalid window function name: {func}"));
        if kind.takes_column() && column.is_none() {
            panic!("window function `{func}` requires a column argument");
        }
        let spec = WindowSpec {
            func: kind,
            column: column.map(|s| s.to_string()),
            partition_by: partition_by.iter().map(|s| s.to_string()).collect(),
            order_by: order_by
                .iter()
                .map(|(c, d)| {
                    (
                        c.to_string(),
                        if *d {
                            OrderDirection::Descending
                        } else {
                            OrderDirection::Ascending
                        },
                    )
                })
                .collect(),
            alias: alias.to_string(),
        };
        self.state.windows.push(spec);
        self
    }

    /// Adds a CTE (Common Table Expression) definition to the query (raw mode).
    ///
    /// `#[doc(hidden)]` — called by runtime API users.
    ///
    /// The CTE body is a pre-compiled SQL string with `?` placeholders; its
    /// parameter values are prepended to the query's parameter vector at
    /// execution time so that placeholder ordering remains contiguous.
    ///
    /// **Note**: Raw mode emits `?` placeholders verbatim and does not convert
    /// them to provider-specific syntax (`$N` on PostgreSQL). For
    /// provider-correct placeholders, use `with_cte_typed` (via
    /// `linq!(with ...)`).
    #[doc(hidden)]
    pub fn with_cte_internal(
        mut self,
        name: &str,
        sql: &str,
        params: Vec<DbValue>,
        columns: &'static [&'static str],
    ) -> Self {
        let cte = CteSpec {
            name: name.to_string(),
            sql: sql.to_string(),
            table: String::new(),
            where_expr: None,
            params,
            columns: columns.iter().map(|s| s.to_string()).collect(),
        };
        self.state.ctes.push(cte);
        self
    }

    /// Adds a typed CTE definition (typed mode), used by `linq!(with ...)`.
    ///
    /// `#[doc(hidden)]` — called by `linq!(with name as |e: T| ...)` expansion.
    ///
    /// The CTE body `SELECT * FROM <table> WHERE <where_expr>` is compiled at
    /// `to_sql_with` time using the provider's placeholder syntax, ensuring
    /// correct `$N` numbering on PostgreSQL and `?` on SQLite/MySQL.
    ///
    /// Parameter values are extracted from `where_expr` via
    /// `collect_bool_expr_values` and stored in `params` so that `all_params()`
    /// returns them in the correct order (CTE params first).
    #[doc(hidden)]
    pub fn with_cte_typed(mut self, name: &str, table: &str, where_expr: BoolExpr) -> Self {
        let params = collect_bool_expr_values(&where_expr);
        let cte = CteSpec {
            name: name.to_string(),
            sql: String::new(),
            table: table.to_string(),
            where_expr: Some(where_expr),
            params,
            columns: Vec::new(),
        };
        self.state.ctes.push(cte);
        self
    }

    /// Changes the FROM clause to reference a CTE name (or any table/subquery).
    ///
    /// Used in combination with `with_cte_internal` to query from a CTE:
    /// ```ignore
    /// builder.with_cte_internal("cte", "SELECT ...", params, &[])
    ///        .from_cte("cte")
    /// ```
    #[doc(hidden)]
    pub fn from_cte(mut self, name: &str) -> Self {
        self.state.from = name.to_string();
        self
    }

    // -------------------------------------------------------------------
    // Aggregate terminal methods
    // -------------------------------------------------------------------

    /// Executes a SUM aggregation query.
    ///
    /// `#[doc(hidden)]` — called by `linq!(sum b.views)` expansion.
    #[doc(hidden)]
    pub async fn sum_internal(self, column: &'static str) -> EFResult<f64> {
        let mut state = self.state.clone();
        state.aggregate = Some("SUM".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            first.trim().parse::<f64>().map_err(|_| {
                crate::error::EFError::TypeConversion("SUM result is not f64".to_string())
            })
        } else {
            Ok(0.0)
        }
    }

    /// Executes an AVG aggregation query.
    ///
    /// `#[doc(hidden)]` — called by `linq!(avg b.rating)` expansion.
    #[doc(hidden)]
    pub async fn avg_internal(self, column: &'static str) -> EFResult<f64> {
        let mut state = self.state.clone();
        state.aggregate = Some("AVG".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            first.trim().parse::<f64>().map_err(|_| {
                crate::error::EFError::TypeConversion("AVG result is not f64".to_string())
            })
        } else {
            Ok(0.0)
        }
    }

    /// Executes a MIN aggregation query, returning the typed result.
    ///
    /// `#[doc(hidden)]` — called by `linq!(min b.rating)` expansion. The target
    /// type `V` is inferred from the call site (e.g. `let v: i64 = ...`).
    #[doc(hidden)]
    pub async fn min_internal<V>(self, column: &'static str) -> EFResult<Option<V>>
    where
        V: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let mut state = self.state.clone();
        state.aggregate = Some("MIN".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        convert_aggregate_cell::<V>(rows)
    }

    /// Executes a MAX aggregation query, returning the typed result.
    ///
    /// `#[doc(hidden)]` — called by `linq!(max b.rating)` expansion. The target
    /// type `V` is inferred from the call site (e.g. `let v: i64 = ...`).
    #[doc(hidden)]
    pub async fn max_internal<V>(self, column: &'static str) -> EFResult<Option<V>>
    where
        V: TryFrom<DbValue, Error = DbValueConvertError>,
    {
        let mut state = self.state.clone();
        state.aggregate = Some("MAX".to_string());
        state.aggregate_column = Some(column.to_string());
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        convert_aggregate_cell::<V>(rows)
    }

    // -------------------------------------------------------------------
    // Terminal methods
    // -------------------------------------------------------------------

    /// Projects to named columns and returns raw row values.
    ///
    /// `#[doc(hidden)]` — called by `linq!(select (b.id, b.title))` expansion.
    #[doc(hidden)]
    pub fn select_internal(self, columns: &'static [&'static str]) -> SelectQueryBuilder<T> {
        let mut state = self.state.clone();
        state.projected_columns = Some(columns.iter().map(|s| s.to_string()).collect());
        SelectQueryBuilder {
            state,
            provider: self.provider,
            _phantom: PhantomData,
        }
    }

    // -------------------------------------------------------------------
    // Terminal methods
    // -------------------------------------------------------------------

    /// Builds the SQL string for this query.
    pub fn to_sql(&self) -> String {
        // G5: Resolve subquery specs using entity metadata.
        let mut state = self.state.clone();
        if let Some(ref mut expr) = state.where_expr {
            if has_subqueries(expr) {
                let meta = T::entity_meta();
                resolve_subqueries(expr, &meta);
            }
        }
        if let Some(provider) = &self.provider {
            let gen = provider.sql_generator();
            state.to_sql_with(gen)
        } else {
            state.to_sql()
        }
    }

    fn compile_sql(&self) -> (String, Vec<DbValue>) {
        (self.to_sql(), self.state.all_params())
    }

    fn compile_state_sql(state: &QueryState, provider: &Arc<dyn IDatabaseProvider>) -> String {
        let gen = provider.sql_generator();
        // G5: Resolve subquery specs before SQL compilation.
        let mut resolved = state.clone();
        if let Some(ref mut expr) = resolved.where_expr {
            if has_subqueries(expr) {
                let meta = T::entity_meta();
                resolve_subqueries(expr, &meta);
            }
        }
        resolved.to_sql_with(gen)
    }

    /// Executes the query and returns all matching entities.
    pub async fn to_list(self) -> EFResult<Vec<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let includes = self.state.includes.clone();
        let lazy_loading = self.lazy_loading_enabled;
        let (sql, params) = self.compile_sql();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder. Use DbSet::query() or attach a provider."
                    .to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        let mut entities = crate::entity::materialize_entities::<T>(&rows)?;
        if !includes.is_empty() {
            crate::navigation_loader::load_includes(
                &mut entities,
                &includes,
                &**provider,
                self.filter_map.as_deref(),
            )
            .await?;
        }
        // When lazy loading is enabled and no explicit includes were
        // requested, attach a LazyContext to each navigation container on
        // every materialized entity. The user can then call
        // `nav.load().await` to trigger on-demand loading.
        if lazy_loading && includes.is_empty() {
            let provider_arc = Arc::clone(provider);
            let filter_map = self.filter_map.clone();
            for entity in &mut entities {
                entity.attach_lazy_contexts(Arc::clone(&provider_arc), filter_map.clone(), 0);
            }
        }
        Ok(entities)
    }

    /// Executes the query and eagerly loads included navigations.
    pub async fn to_list_with_includes(self) -> EFResult<Vec<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        self.to_list().await
    }

    /// Executes the query and returns the first matching entity.
    pub async fn first(self) -> EFResult<T>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(1).to_list().await?;
        results
            .pop()
            .ok_or_else(|| crate::error::EFError::NotFound("Entity not found".to_string()))
    }

    /// Executes the query and returns the first matching entity or None.
    pub async fn first_or_default(self) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(1).to_list().await?;
        Ok(results.pop())
    }

    /// Executes a COUNT query.
    pub async fn count(self) -> EFResult<i64> {
        let mut state = self.state.clone();
        state.is_count = true;
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first_row) = rows.first() {
            if let Some(first_val) = first_row.first() {
                return first_val.trim().parse::<i64>().map_err(|e| {
                    crate::error::EFError::TypeConversion(format!(
                        "COUNT result '{}' is not i64: {}",
                        first_val, e
                    ))
                });
            }
        }
        Ok(0)
    }

    /// Checks if any entities match the query.
    pub async fn any(self) -> EFResult<bool> {
        let mut state = self.state.clone();
        state.is_exists = true;
        state.limit = Some(1);
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let sql = Self::compile_state_sql(&state, provider);
        let params = state.all_params();
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        Ok(!rows.is_empty())
    }

    // -------------------------------------------------------------------
    // Additional LINQ terminal methods
    // -------------------------------------------------------------------

    /// Executes the query and returns the last matching entity (reverses
    /// ordering, then takes 1). Errors if no rows match.
    pub async fn last(self) -> EFResult<T>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.last_or_default().await?;
        results
            .take()
            .ok_or_else(|| crate::error::EFError::NotFound("Entity not found".to_string()))
    }

    /// Executes the query and returns the last matching entity or `None`.
    ///
    /// When the caller has set explicit `order_by` clauses, their directions
    /// are reversed and `take(1)` returns the last row under that ordering.
    /// When no ordering is set, a default `ORDER BY <pk> DESC` is injected so
    /// that "last" has deterministic semantics (matches the original design
    /// in the v0.4 plan §4 阶段 4). Errors if the entity has no primary key
    /// and no explicit ordering was provided.
    pub async fn last_or_default(mut self) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        if self.state.orderings.is_empty() {
            let meta = T::entity_meta();
            let pk_col = meta
                .primary_keys
                .first()
                .map(|s| s.as_ref())
                .or_else(|| {
                    meta.properties
                        .iter()
                        .find(|p| p.is_primary_key)
                        .map(|p| p.column_name.as_ref())
                })
                .ok_or_else(|| {
                    crate::error::EFError::Query(format!(
                        "last_or_default requires a primary key on {} when no explicit ordering is set",
                        std::any::type_name::<T>()
                    ))
                })?;
            self.state
                .orderings
                .push(OrderBy::new(pk_col.to_string(), OrderDirection::Descending));
        } else {
            // Reverse existing orderings to get the "last" row.
            for o in &mut self.state.orderings {
                o.direction = match o.direction {
                    OrderDirection::Ascending => OrderDirection::Descending,
                    OrderDirection::Descending => OrderDirection::Ascending,
                };
            }
        }
        let mut results = self.take(1).to_list().await?;
        Ok(results.pop())
    }

    /// Executes the query and returns the only matching entity. Errors if
    /// there are 0 or 2+ results.
    pub async fn single(self) -> EFResult<T>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(2).to_list().await?;
        if results.len() > 1 {
            return Err(crate::error::EFError::Query(
                "Sequence contains more than one element".to_string(),
            ));
        }
        results.pop().ok_or_else(|| {
            crate::error::EFError::NotFound("Sequence contains no elements".to_string())
        })
    }

    /// Executes the query and returns the only matching entity, or `None` if
    /// empty. Errors if there are 2+ results.
    pub async fn single_or_default(self) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        let mut results = self.take(2).to_list().await?;
        if results.len() > 1 {
            return Err(crate::error::EFError::Query(
                "Sequence contains more than one element".to_string(),
            ));
        }
        Ok(results.pop())
    }

    /// Executes a COUNT query and returns the result as `i64`. Alias for
    /// `count()` — in .NET LINQ, `LongCount` returns `long` while `Count`
    /// returns `int`; in Rust both are `i64`.
    pub async fn long_count(self) -> EFResult<i64> {
        self.count().await
    }

    /// Determines whether all elements in the sequence satisfy a predicate.
    /// The predicate is applied in Rust after loading the entities.
    pub async fn all<F>(self, predicate: F) -> EFResult<bool>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
        F: Fn(&T) -> bool,
    {
        let items = self.to_list().await?;
        Ok(items.iter().all(predicate))
    }

    /// Determines whether the sequence contains an entity with the given
    /// primary key value.
    pub async fn contains(self, id: impl Into<DbValue>) -> EFResult<bool>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
    {
        self.find(id).await.map(|opt| opt.is_some())
    }

    /// Projects each entity into a key-value pair and collects into a
    /// `HashMap<K, T>`. The key selector closure extracts the key from each
    /// entity.
    pub async fn to_dictionary<K, F>(
        self,
        key_selector: F,
    ) -> EFResult<std::collections::HashMap<K, T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot + ILazyInit,
        K: std::hash::Hash + Eq,
        F: Fn(&T) -> K,
    {
        let items = self.to_list().await?;
        let mut map = std::collections::HashMap::with_capacity(items.len());
        for item in items {
            let key = key_selector(&item);
            map.insert(key, item);
        }
        Ok(map)
    }

    // -------------------------------------------------------------------
    // Bulk operations (ExecuteUpdate / ExecuteDelete)
    // -------------------------------------------------------------------

    /// Prepares a bulk update operation.
    pub fn execute_update(self) -> ExecuteUpdateBuilder<T> {
        ExecuteUpdateBuilder {
            state: self.state.clone(),
            updates: Vec::new(),
            provider: self.provider.clone(),
            _phantom: PhantomData,
        }
    }

    /// Executes a bulk delete operation.
    pub async fn execute_delete(self) -> EFResult<u64> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let gen = provider.sql_generator();
        // G5: Resolve subqueries before compiling WHERE clause.
        let mut resolved_expr = self.state.where_expr.clone();
        if let Some(ref mut expr) = resolved_expr {
            if has_subqueries(expr) {
                let meta = T::entity_meta();
                resolve_subqueries(expr, &meta);
            }
        }
        let where_clause = if let Some(ref expr) = resolved_expr {
            let mut param_idx = 1usize;
            compile_bool_expr(expr, gen, &mut param_idx)
        } else {
            build_where_clauses(&self.state.filters, gen)
        };
        let sql = if where_clause.is_empty() {
            format!("DELETE FROM {}", self.state.from)
        } else {
            format!("DELETE FROM {} WHERE {}", self.state.from, where_clause)
        };
        let params = self.state.all_params();
        let mut conn = provider.get_connection().await?;
        conn.execute(&sql, &params).await
    }
}

// ---------------------------------------------------------------------------
// ExecuteUpdate builder
// ---------------------------------------------------------------------------

/// Builder for bulk update operations.
#[derive(Clone)]
pub struct ExecuteUpdateBuilder<T: IEntityType> {
    state: QueryState,
    updates: Vec<(String, DbValue)>,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    _phantom: PhantomData<T>,
}

impl<T: IEntityType> ExecuteUpdateBuilder<T> {
    /// Sets a named column to a DbValue.
    ///
    /// `#[doc(hidden)]` — called by `linq!(set b.views, 10; execute_update)`
    /// expansion.
    #[doc(hidden)]
    pub fn set_column_internal(mut self, column: &'static str, value: impl Into<DbValue>) -> Self {
        self.updates.push((column.to_string(), value.into()));
        self
    }

    /// Returns the generated SQL.
    pub fn to_sql(&self) -> String {
        let gen: &'static dyn crate::provider::ISqlGenerator = self
            .provider
            .as_ref()
            .map(|p| p.sql_generator())
            .unwrap_or(&PortablePlaceholderGenerator);
        let mut param_idx = 1usize;
        let sets: Vec<String> = self
            .updates
            .iter()
            .map(|(col, _)| {
                let ph = gen.parameter_placeholder(param_idx);
                param_idx += 1;
                format!("{} = {}", col, ph)
            })
            .collect();
        let where_clause = if let Some(ref expr) = self.state.where_expr {
            let mut param_idx = param_idx;
            compile_bool_expr(expr, gen, &mut param_idx)
        } else {
            build_where_clause_with_offset(&self.state.filters, gen, param_idx)
        };
        if where_clause.is_empty() {
            format!("UPDATE {} SET {}", self.state.from, sets.join(", "))
        } else {
            format!(
                "UPDATE {} SET {} WHERE {}",
                self.state.from,
                sets.join(", "),
                where_clause
            )
        }
    }

    /// Returns params for this bulk update.
    pub fn params(&self) -> Vec<DbValue> {
        let mut params: Vec<DbValue> = self.updates.iter().map(|(_, v)| v.clone()).collect();
        params.extend_from_slice(&self.state.parameters);
        params
    }

    /// Executes the bulk update.
    pub async fn execute(self) -> EFResult<u64> {
        let sql = self.to_sql();
        let params = self.params();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to ExecuteUpdateBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        conn.execute(&sql, &params).await
    }
}

// ---------------------------------------------------------------------------
// Select query builder (for projections)
// ---------------------------------------------------------------------------

/// A query builder for projected column results.
#[derive(Clone)]
pub struct SelectQueryBuilder<T: IEntityType> {
    state: QueryState,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    _phantom: PhantomData<T>,
}

impl<T: IEntityType> SelectQueryBuilder<T> {
    /// Returns the generated SQL.
    pub fn to_sql(&self) -> String {
        if let Some(provider) = &self.provider {
            let gen = provider.sql_generator();
            self.state.to_sql_with(gen)
        } else {
            self.state.to_sql()
        }
    }

    /// Executes the projection query and returns raw column values per row.
    pub async fn to_list(self) -> EFResult<Vec<Vec<String>>> {
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::EFError::Configuration(
                "No provider attached to SelectQueryBuilder.".to_string(),
            )
        })?;
        let gen = provider.sql_generator();
        let sql = self.state.to_sql_with(gen);
        let params = self.state.all_params();
        let mut conn = provider.get_connection().await?;
        conn.query(&sql, &params).await
    }

    // -------------------------------------------------------------------
    // G3: Strongly-typed projection terminal methods.
    //
    // Each `to_list_typed_n::<V0, ...>` method executes the projection
    // query, then parses each column value via `ParseFromDb` into the
    // corresponding type parameter, returning `Vec<(V0, ...)>`.
    // -------------------------------------------------------------------

    async fn fetch_rows(self) -> EFResult<Vec<Vec<String>>> {
        self.to_list().await
    }

    /// Single-column typed projection → `Vec<V0>`.
    pub async fn to_list_typed_1<V0>(self) -> EFResult<Vec<V0>>
    where
        V0: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                parse_column::<V0>(row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row has no columns".into())
                })?)
            })
            .collect()
    }

    /// Two-column typed projection → `Vec<(V0, V1)>`.
    pub async fn to_list_typed_2<V0, V1>(self) -> EFResult<Vec<(V0, V1)>>
    where
        V0: ParseFromDb,
        V1: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 0".into())
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 1".into())
                })?;
                Ok((parse_column::<V0>(c0)?, parse_column::<V1>(c1)?))
            })
            .collect()
    }

    /// Three-column typed projection → `Vec<(V0, V1, V2)>`.
    pub async fn to_list_typed_3<V0, V1, V2>(self) -> EFResult<Vec<(V0, V1, V2)>>
    where
        V0: ParseFromDb,
        V1: ParseFromDb,
        V2: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 0".into())
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 1".into())
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 2".into())
                })?;
                Ok((
                    parse_column::<V0>(c0)?,
                    parse_column::<V1>(c1)?,
                    parse_column::<V2>(c2)?,
                ))
            })
            .collect()
    }

    /// Four-column typed projection → `Vec<(V0, V1, V2, V3)>`.
    pub async fn to_list_typed_4<V0, V1, V2, V3>(self) -> EFResult<Vec<(V0, V1, V2, V3)>>
    where
        V0: ParseFromDb,
        V1: ParseFromDb,
        V2: ParseFromDb,
        V3: ParseFromDb,
    {
        let rows = self.fetch_rows().await?;
        rows.into_iter()
            .map(|row| {
                let c0 = row.first().ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 0".into())
                })?;
                let c1 = row.get(1).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 1".into())
                })?;
                let c2 = row.get(2).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 2".into())
                })?;
                let c3 = row.get(3).ok_or_else(|| {
                    crate::error::EFError::Query("projection row missing column 3".into())
                })?;
                Ok((
                    parse_column::<V0>(c0)?,
                    parse_column::<V1>(c1)?,
                    parse_column::<V2>(c2)?,
                    parse_column::<V3>(c3)?,
                ))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Marker trait implemented by query source types (`QueryBuilder<T>`, `DbSet<T>`).
///
/// `LinqSource` enables the `linq!` macro to accept untyped closures
/// (`|b| ...`) when the source expression carries entity type information
/// via turbofish (e.g. `ctx.set::<Blog>()`). The macro extracts the type
/// from the source and generates a typed closure internally.
pub trait LinqSource {}

impl<T: IEntityType> LinqSource for QueryBuilder<T> {}
impl<T: IEntityType> LinqSource for crate::db_set::DbSet<T> {}

/// Parses a `&str` column value from the database into a Rust type.
///
/// Unlike `FromStr`, this trait handles database-specific representations:
/// - `bool`: accepts `"0"`/`"1"` (SQLite/MySQL) as well as `"true"`/`"false"`
/// - Numeric types: fall back to `FromStr`
/// - `String`: returns the value as-is
pub trait ParseFromDb: Sized {
    fn parse_from_db(s: &str) -> EFResult<Self>;
}

impl ParseFromDb for bool {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        match s {
            "1" | "true" | "t" | "TRUE" | "T" => Ok(true),
            "0" | "false" | "f" | "FALSE" | "F" | "" => Ok(false),
            _ => Err(crate::error::EFError::Query(format!(
                "failed to parse '{}' as bool",
                s
            ))),
        }
    }
}

impl ParseFromDb for i32 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as i32: {}", s, e))
        })
    }
}

impl ParseFromDb for i64 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as i64: {}", s, e))
        })
    }
}

impl ParseFromDb for f64 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as f64: {}", s, e))
        })
    }
}

impl ParseFromDb for f32 {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        s.parse().map_err(|e| {
            crate::error::EFError::Query(format!("failed to parse '{}' as f32: {}", s, e))
        })
    }
}

impl ParseFromDb for String {
    fn parse_from_db(s: &str) -> EFResult<Self> {
        Ok(s.to_string())
    }
}

/// Parses a `&str` column value into `V` via `ParseFromDb`.
fn parse_column<V: ParseFromDb>(s: &str) -> EFResult<V> {
    V::parse_from_db(s)
}

fn filters_to_and_expr(filters: &[FilterCondition]) -> BoolExpr {
    filters
        .iter()
        .cloned()
        .map(BoolExpr::Filter)
        .reduce(|acc, f| BoolExpr::And(Box::new(acc), Box::new(f)))
        .unwrap_or(BoolExpr::Raw("1=1".to_string()))
}

pub(crate) fn compile_bool_expr(
    expr: &BoolExpr,
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
) -> String {
    match expr {
        BoolExpr::Filter(f) => {
            let placeholders: Vec<String> = (0..f.param_count())
                .map(|i| gen.parameter_placeholder(*param_idx + i))
                .collect();
            *param_idx += f.param_count();
            f.to_sql(&placeholders)
        }
        BoolExpr::Raw(sql) => sql.clone(),
        BoolExpr::And(a, b) => format!(
            "({}) AND ({})",
            compile_bool_expr(a, gen, param_idx),
            compile_bool_expr(b, gen, param_idx)
        ),
        BoolExpr::Or(a, b) => format!(
            "({}) OR ({})",
            compile_bool_expr(a, gen, param_idx),
            compile_bool_expr(b, gen, param_idx)
        ),
        BoolExpr::Not(inner) => format!("NOT ({})", compile_bool_expr(inner, gen, param_idx)),
        BoolExpr::Exists(spec) => compile_subquery(spec, gen, param_idx, false),
        BoolExpr::NotExists(spec) => compile_subquery(spec, gen, param_idx, true),
        BoolExpr::InSubquery(spec) => compile_in_subquery(spec, gen, param_idx, false),
        BoolExpr::NotInSubquery(spec) => compile_in_subquery(spec, gen, param_idx, true),
    }
}

/// G5: Compiles a `SubquerySpec` into `EXISTS (SELECT 1 FROM ...)` SQL.
fn compile_subquery(
    spec: &SubquerySpec,
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
    negated: bool,
) -> String {
    let related_tbl = gen.quote_identifier(&spec.related_table);
    let fk_col = gen.quote_identifier(&spec.fk_column);
    let outer_tbl = gen.quote_identifier(&spec.outer_table);
    let outer_pk = gen.quote_identifier(&spec.outer_pk_column);

    let mut sql = if negated {
        format!("NOT EXISTS (SELECT 1 FROM {related_tbl} WHERE {related_tbl}.{fk_col} = {outer_tbl}.{outer_pk}")
    } else {
        format!("EXISTS (SELECT 1 FROM {related_tbl} WHERE {related_tbl}.{fk_col} = {outer_tbl}.{outer_pk}")
    };

    if let Some(pred) = &spec.predicate {
        let pred_sql = compile_bool_expr(pred, gen, param_idx);
        sql.push_str(&format!(" AND {pred_sql}"));
    }
    sql.push(')');
    sql
}

/// v1.1: Compiles an `InSubquerySpec` into
/// `column IN (SELECT projection FROM source_table [WHERE predicate])` SQL.
fn compile_in_subquery(
    spec: &InSubquerySpec,
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
    negated: bool,
) -> String {
    let outer_col = gen.quote_identifier(&spec.outer_column);
    let src_tbl = gen.quote_identifier(&spec.source_table);
    let proj_col = gen.quote_identifier(&spec.projection_column);

    let kw = if negated { "NOT IN" } else { "IN" };
    let mut sql = format!("{outer_col} {kw} (SELECT {proj_col} FROM {src_tbl}");

    if let Some(pred) = &spec.predicate {
        let pred_sql = compile_bool_expr(pred, gen, param_idx);
        sql.push_str(&format!(" WHERE {pred_sql}"));
    }
    sql.push(')');
    sql
}

/// Walks a `BoolExpr` tree and collects inline parameter values carried by
/// self-contained `FilterCondition`s (those produced by `linq!(filter |b: T| ...)`
/// Form C). Returns an empty vec for expressions whose values are already
/// tracked in `QueryState::parameters` (in-builder conditions).
pub(crate) fn collect_bool_expr_values(expr: &BoolExpr) -> Vec<DbValue> {
    match expr {
        BoolExpr::Filter(f) => f.values().to_vec(),
        BoolExpr::Raw(_) => Vec::new(),
        BoolExpr::And(a, b) | BoolExpr::Or(a, b) => {
            let mut v = collect_bool_expr_values(a);
            v.extend(collect_bool_expr_values(b));
            v
        }
        BoolExpr::Not(inner) => collect_bool_expr_values(inner),
        BoolExpr::Exists(spec) | BoolExpr::NotExists(spec) => spec
            .predicate
            .as_ref()
            .map(|p| collect_bool_expr_values(p))
            .unwrap_or_default(),
        BoolExpr::InSubquery(spec) | BoolExpr::NotInSubquery(spec) => spec
            .predicate
            .as_ref()
            .map(|p| collect_bool_expr_values(p))
            .unwrap_or_default(),
    }
}

/// G5: Resolves `SubquerySpec` table/column fields by looking up navigation
/// metadata from the outer entity's `EntityTypeMeta`. Must be called before
/// `compile_bool_expr` when the expression tree contains `Exists`/`NotExists`.
pub(crate) fn resolve_subqueries(expr: &mut BoolExpr, outer_meta: &EntityTypeMeta) {
    match expr {
        BoolExpr::And(a, b) | BoolExpr::Or(a, b) => {
            resolve_subqueries(a, outer_meta);
            resolve_subqueries(b, outer_meta);
        }
        BoolExpr::Not(inner) => resolve_subqueries(inner, outer_meta),
        BoolExpr::Exists(spec) | BoolExpr::NotExists(spec) => {
            resolve_subquery_spec(spec, outer_meta);
            if let Some(pred) = &mut spec.predicate {
                // The predicate references the related entity, but we don't
                // have its metadata here. Predicate fields are compiled as
                // raw column names (e.g. "published"), which works for
                // single-table subqueries.
                let _ = pred;
            }
        }
        _ => {}
    }
}

/// Returns `true` if the expression tree contains any `Exists`/`NotExists`
/// subquery nodes. Used to avoid the `T::entity_meta()` call (which may be
/// `unimplemented!()` in unit tests) when no subqueries are present.
fn has_subqueries(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Exists(_) | BoolExpr::NotExists(_) => true,
        BoolExpr::And(a, b) | BoolExpr::Or(a, b) => has_subqueries(a) || has_subqueries(b),
        BoolExpr::Not(inner) => has_subqueries(inner),
        _ => false,
    }
}

fn resolve_subquery_spec(spec: &mut SubquerySpec, outer_meta: &EntityTypeMeta) {
    // Find the navigation by field name
    let nav = outer_meta
        .navigations
        .iter()
        .find(|n| n.field_name.as_ref() == spec.navigation_field);

    if let Some(nav) = nav {
        // Related table: prefer the navigation's `related_table` (actual DB
        // table name, e.g. "posts"); fall back to the related type name.
        spec.related_table = nav
            .related_table
            .as_ref()
            .map(|s| s.as_ref().to_string())
            .unwrap_or_else(|| spec.related_type_name.clone());

        // FK column on the related/dependent table (e.g. "blog_id").
        if let Some(fk) = &nav.fk_column {
            spec.fk_column = fk.as_ref().to_string();
        } else if let Some(fk) = &nav.foreign_key_field {
            spec.fk_column = fk.as_ref().to_string();
        }

        // Outer PK: prefer the navigation's `referenced_key_column`, fall
        // back to the outer entity's first primary key.
        if let Some(pk) = &nav.referenced_key_column {
            spec.outer_pk_column = pk.as_ref().to_string();
        } else if let Some(pk) = outer_meta.primary_keys.first() {
            spec.outer_pk_column = pk.as_ref().to_string();
        }

        // Outer table: from the outer entity's table_name
        spec.outer_table = outer_meta.table_name.to_string();
    }
}

fn build_where_clauses(
    filters: &[FilterCondition],
    gen: &dyn crate::provider::ISqlGenerator,
) -> String {
    build_where_clause_with_offset(filters, gen, 1)
}

fn build_where_clause_with_offset(
    filters: &[FilterCondition],
    gen: &dyn crate::provider::ISqlGenerator,
    start_index: usize,
) -> String {
    if filters.is_empty() {
        return String::new();
    }
    let mut param_idx = start_index;
    let clauses: Vec<String> = filters
        .iter()
        .map(|f| {
            let placeholders: Vec<String> = (0..f.param_count())
                .map(|i| gen.parameter_placeholder(param_idx + i))
                .collect();
            param_idx += f.param_count();
            f.to_sql(&placeholders)
        })
        .collect();
    clauses.join(" AND ")
}

// ---------------------------------------------------------------------------
// LINQ string pattern helpers
// ---------------------------------------------------------------------------

/// Builds a `%value%` LIKE pattern (EFCore `Contains`).
pub fn like_contains(value: impl AsRef<str>) -> String {
    format!("%{}%", value.as_ref())
}

/// Builds a `value%` LIKE pattern (EFCore `StartsWith`).
pub fn like_starts_with(value: impl AsRef<str>) -> String {
    format!("{}%", value.as_ref())
}

/// Builds a `%value` LIKE pattern (EFCore `EndsWith`).
pub fn like_ends_with(value: impl AsRef<str>) -> String {
    format!("%{}", value.as_ref())
}

// ---------------------------------------------------------------------------
// IQueryable trait
// ---------------------------------------------------------------------------

/// Trait representing a queryable data source.
pub trait IQueryable<T: IEntityType> {
    fn query(&self) -> QueryBuilder<T>;
}
