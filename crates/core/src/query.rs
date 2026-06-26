//! Query builder ??LINQ-style chainable query API.
//!
//! Accumulates filter conditions, orderings, pagination, includes, and
//! projection metadata through a fluent interface. Terminal methods
//! (`to_list`, `first`, `count`, etc.) produce real SQL that can be
//! executed against a database provider.

use crate::entity::{IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, INavigationSetter};
use crate::error::EFResult;
use crate::provider::{DbValue, DbValueConvertError, IDatabaseProvider};
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
    /// Recursively compiles the expression into a SQL fragment.
    ///
    /// Bound parameters (for `Compare` variants) are pushed into `params` in
    /// left-to-right order and represented as `?` placeholders in the output.
    pub fn to_sql(&self, params: &mut Vec<DbValue>) -> String {
        match self {
            Self::Compare {
                agg,
                col,
                op,
                value,
            } => {
                params.push(value.clone());
                format!("{}({}) {} ?", agg.sql_name(), col, op.sql_name())
            }
            Self::And(left, right) => {
                format!("({} AND {})", left.to_sql(params), right.to_sql(params))
            }
            Self::Or(left, right) => {
                format!("({} OR {})", left.to_sql(params), right.to_sql(params))
            }
            Self::Not(inner) => {
                format!("NOT ({})", inner.to_sql(params))
            }
            Self::CompareAgg {
                left_agg,
                left_col,
                op,
                right_agg,
                right_col,
            } => {
                format!(
                    "{}({}) {} {}({})",
                    left_agg.sql_name(),
                    left_col,
                    op.sql_name(),
                    right_agg.sql_name(),
                    right_col
                )
            }
        }
    }
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
    /// HAVING conditions.
    pub havings: Vec<String>,
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
            format!("SELECT {}{}", distinct_kw, cols.join(", "))
        } else {
            format!("SELECT {}*", distinct_kw)
        };

        let mut sql = format!("{} FROM {}", select, self.from);

        // JOINs
        for join in &self.joins {
            sql.push_str(&format!(" {}", join.to_sql()));
        }

        // WHERE
        if let Some(ref expr) = self.where_expr {
            let mut param_idx = 1usize;
            sql.push_str(&format!(
                " WHERE {}",
                compile_bool_expr(expr, gen, &mut param_idx)
            ));
        } else if !self.filters.is_empty() {
            sql.push_str(&format!(
                " WHERE {}",
                build_where_clauses(&self.filters, gen)
            ));
        }

        // GROUP BY
        if !self.group_bys.is_empty() {
            sql.push_str(&format!(" GROUP BY {}", self.group_bys.join(", ")));
        }

        // HAVING
        if !self.havings.is_empty() {
            sql.push_str(&format!(" HAVING {}", self.havings.join(" AND ")));
        }

        // ORDER BY
        if !self.orderings.is_empty() {
            let ords: Vec<String> = self.orderings.iter().map(|o| o.to_sql()).collect();
            sql.push_str(&format!(" ORDER BY {}", ords.join(", ")));
        }

        // LIMIT / OFFSET
        match (self.limit, self.offset) {
            (Some(limit), Some(offset)) => {
                sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));
            }
            (Some(limit), None) => {
                sql.push_str(&format!(" LIMIT {}", limit));
            }
            (None, Some(offset)) => {
                sql.push_str(&format!(" OFFSET {}", offset));
            }
            (None, None) => {}
        }

        sql
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
    fn pagination(&self, _: Option<usize>, _: Option<usize>) -> String {
        String::new()
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
    _phantom: PhantomData<T>,
}

impl<T: IEntityType> QueryBuilder<T> {
    /// Creates a new QueryBuilder for a given table (without provider ??SQL-only).
    pub fn new(table_name: impl Into<String>) -> Self {
        Self {
            state: QueryState::new(table_name),
            provider: None,
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
            _phantom: PhantomData,
        }
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

    // -------------------------------------------------------------------
    // Chainable methods (each returns Self with accumulated state)
    // -------------------------------------------------------------------

    /// Finds an entity by its single primary key. Uses the entity's PK
    /// metadata — no longer hardcodes `"id"`.
    pub async fn find(self, id: impl Into<DbValue>) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
    {
        for (col, val) in keys {
            self = self.filter_column(col, "=", val.clone());
        }
        self.first_or_default().await
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
        let db_val = value.into();
        self.state.parameters.push(db_val);
        self.state
            .havings
            .push(format!("{}({}) {} ?", agg, column, op));
        self
    }

    /// Adds a HAVING condition from a `HavingExpr` AST.
    ///
    /// `#[doc(hidden)]` — called by `linq!(having <expr>)` expansion when the
    /// having clause contains boolean combinations (`AND`/`OR`/`NOT`) or
    /// aggregate-versus-aggregate comparisons. The expression is compiled to
    /// SQL via [`HavingExpr::to_sql`], with bound parameters pushed to
    /// `state.parameters`.
    #[doc(hidden)]
    pub fn having_expr_internal(mut self, expr: HavingExpr) -> Self {
        let mut params = Vec::new();
        let sql = expr.to_sql(&mut params);
        self.state.havings.push(sql);
        self.state.parameters.extend(params);
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
        let params = state.params().to_vec();
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
        let params = state.params().to_vec();
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
        let params = state.params().to_vec();
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
        let params = state.params().to_vec();
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
        if let Some(provider) = &self.provider {
            let gen = provider.sql_generator();
            self.state.to_sql_with(&*gen)
        } else {
            self.state.to_sql()
        }
    }

    fn compile_sql(&self) -> (String, Vec<DbValue>) {
        (self.to_sql(), self.state.params().to_vec())
    }

    fn compile_state_sql(state: &QueryState, provider: &Arc<dyn IDatabaseProvider>) -> String {
        let gen = provider.sql_generator();
        state.to_sql_with(&*gen)
    }

    /// Executes the query and returns all matching entities.
    pub async fn to_list(self) -> EFResult<Vec<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
    {
        let includes = self.state.includes.clone();
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
            crate::navigation_loader::load_includes(&mut entities, &includes, &**provider).await?;
        }
        Ok(entities)
    }

    /// Executes the query and eagerly loads included navigations.
    pub async fn to_list_with_includes(self) -> EFResult<Vec<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
    {
        self.to_list().await
    }

    /// Executes the query and returns the first matching entity.
    pub async fn first(self) -> EFResult<T>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
    {
        let mut results = self.take(1).to_list().await?;
        results
            .pop()
            .ok_or_else(|| crate::error::EFError::NotFound("Entity not found".to_string()))
    }

    /// Executes the query and returns the first matching entity or None.
    pub async fn first_or_default(self) -> EFResult<Option<T>>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        let params = state.params().to_vec();
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
        let params = state.params().to_vec();
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
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
        F: Fn(&T) -> bool,
    {
        let items = self.to_list().await?;
        Ok(items.iter().all(predicate))
    }

    /// Determines whether the sequence contains an entity with the given
    /// primary key value.
    pub async fn contains(self, id: impl Into<DbValue>) -> EFResult<bool>
    where
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
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
        let where_clause = if let Some(ref expr) = self.state.where_expr {
            let mut param_idx = 1usize;
            compile_bool_expr(expr, &*gen, &mut param_idx)
        } else {
            build_where_clauses(&self.state.filters, &*gen)
        };
        let sql = if where_clause.is_empty() {
            format!("DELETE FROM {}", self.state.from)
        } else {
            format!("DELETE FROM {} WHERE {}", self.state.from, where_clause)
        };
        let params = self.state.params().to_vec();
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
        let gen = self
            .provider
            .as_ref()
            .map(|p| p.sql_generator())
            .unwrap_or_else(|| Box::new(PortablePlaceholderGenerator));
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
            compile_bool_expr(expr, &*gen, &mut param_idx)
        } else {
            build_where_clause_with_offset(&self.state.filters, &*gen, param_idx)
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
            self.state.to_sql_with(&*gen)
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
        let sql = self.state.to_sql_with(&*gen);
        let params = self.state.params().to_vec();
        let mut conn = provider.get_connection().await?;
        conn.query(&sql, &params).await
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn filters_to_and_expr(filters: &[FilterCondition]) -> BoolExpr {
    filters
        .iter()
        .cloned()
        .map(BoolExpr::Filter)
        .reduce(|acc, f| BoolExpr::And(Box::new(acc), Box::new(f)))
        .unwrap_or(BoolExpr::Raw("1=1".to_string()))
}

fn compile_bool_expr(
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
    }
}

/// Walks a `BoolExpr` tree and collects inline parameter values carried by
/// self-contained `FilterCondition`s (those produced by `linq!(filter |b: T| ...)`
/// Form C). Returns an empty vec for expressions whose values are already
/// tracked in `QueryState::parameters` (in-builder conditions).
fn collect_bool_expr_values(expr: &BoolExpr) -> Vec<DbValue> {
    match expr {
        BoolExpr::Filter(f) => f.values().to_vec(),
        BoolExpr::Raw(_) => Vec::new(),
        BoolExpr::And(a, b) | BoolExpr::Or(a, b) => {
            let mut v = collect_bool_expr_values(a);
            v.extend(collect_bool_expr_values(b));
            v
        }
        BoolExpr::Not(inner) => collect_bool_expr_values(inner),
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
