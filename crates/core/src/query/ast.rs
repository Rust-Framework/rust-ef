//! Query expression AST types.
//!
//! Boolean expression tree (`BoolExpr`), filter conditions, ordering,
//! grouping, having, includes, joins, and supporting types used across
//! the query builder and SQL compiler.

use crate::provider::{DbValue, ISqlGenerator};

use super::expr::SqlExpr;
use super::having_expr::CompareOp;

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
    /// Inline parameter values — the values travel with the tree, so a
    /// `BoolExpr` can be stored, combined and applied without touching
    /// `QueryState`. Empty for value-less operators (`IS NULL`).
    pub(crate) values: Vec<DbValue>,
}

impl FilterCondition {
    /// Creates a condition with a declared parameter count but no values.
    ///
    /// `#[doc(hidden)]` — used by `linq!` expansion for value-less operators
    /// (`IS NULL`). Conditions that bind values must be built with
    /// [`FilterCondition::with_values`], otherwise the generated `?` has
    /// nothing to bind to (`QueryBuilder::apply_query_filter` rejects that
    /// mismatch).
    #[doc(hidden)]
    pub fn new(column: impl Into<String>, operator: impl Into<String>, param_count: usize) -> Self {
        Self {
            column: column.into(),
            operator: operator.into(),
            param_count,
            values: Vec::new(),
        }
    }

    /// Creates a condition carrying its own parameter values. `linq!` uses this
    /// to produce self-contained `BoolExpr` values (predicates, global query
    /// filters) whose values travel with the tree.
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
    /// value-less operators).
    pub fn values(&self) -> &[DbValue] {
        &self.values
    }

    /// Convert to a SQL WHERE fragment using dialect-specific placeholders.
    pub fn to_sql(&self, placeholders: &[String]) -> String {
        match self.operator.as_str() {
            "IS NULL" => format!("{} IS NULL", self.column),
            "IS NOT NULL" => format!("{} IS NOT NULL", self.column),
            // `col IN ()` is not valid SQL; an empty candidate set matches no
            // row, which is what `1 = 0` says.
            "IN" if placeholders.is_empty() => "1 = 0".to_string(),
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

    /// Compiles this condition with provider-specific placeholders, advancing
    /// `param_idx` and collecting its bound values into `params`.
    ///
    /// This is the single compiler used by WHERE, HAVING (via
    /// `HavingExpr::into_bool_expr`) and `CASE WHEN` arms.
    pub(crate) fn compile(
        &self,
        gen: &dyn ISqlGenerator,
        param_idx: &mut usize,
        params: &mut Vec<DbValue>,
    ) -> String {
        let placeholders: Vec<String> = (0..self.param_count)
            .map(|offset| gen.parameter_placeholder(*param_idx + offset))
            .collect();
        *param_idx += self.param_count;
        params.extend(self.values.iter().cloned());
        self.to_sql(&placeholders)
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
#[non_exhaustive]
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
    /// EXISTS (SELECT 1 FROM related_table WHERE related.fk = outer.pk AND `predicate`)
    Exists(Box<SubquerySpec>),
    /// NOT EXISTS (...)
    NotExists(Box<SubquerySpec>),
    /// `column IN (SELECT projection FROM source_table [WHERE predicate])`
    InSubquery(Box<InSubquerySpec>),
    /// `column NOT IN (SELECT projection FROM source_table [WHERE predicate])`
    NotInSubquery(Box<InSubquerySpec>),
    /// A comparison between two scalar expressions, e.g.
    /// `SUM(amount) > ?` (HAVING, lowered from [`HavingExpr`](super::having_expr::HavingExpr))
    /// or `(amount - discount) >= ?`.
    Compare {
        /// Left operand.
        left: Box<SqlExpr>,
        /// Comparison operator.
        op: CompareOp,
        /// Right operand.
        right: Box<SqlExpr>,
    },
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

    /// Builds a scalar comparison node (`left op right`).
    pub fn compare(left: SqlExpr, op: CompareOp, right: SqlExpr) -> Self {
        BoolExpr::Compare {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    pub fn total_param_count(&self) -> usize {
        match self {
            BoolExpr::Filter(f) => f.param_count(),
            BoolExpr::Raw(_) => 0,
            BoolExpr::Compare { left, right, .. } => left.param_count() + right.param_count(),
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

impl FilterCondition {
    /// Returns a copy of this condition with its column qualified by `table`.
    ///
    /// A column that already carries a qualifier (contains a `.`) is left
    /// untouched, so qualifying an already-qualified condition is idempotent.
    /// Used by [`QueryBuilder::filter_joined`](crate::query::QueryBuilder::filter_joined)
    /// so that a typed predicate written for the joined entity filters the
    /// joined table instead of an unqualified column.
    pub(crate) fn qualified(self, table: &str) -> Self {
        let column = if self.column.contains('.') {
            self.column
        } else {
            format!("{table}.{}", self.column)
        };
        let param_count = self.param_count;
        let mut condition = Self::with_values(column, self.operator, self.values);
        condition.param_count = param_count;
        condition
    }
}

/// Qualifies every plain filter column in `expr` with `table`.
///
/// `EXISTS` / `IN (SELECT ...)` subquery nodes and raw SQL fragments are kept
/// as-is: their columns belong to the subquery's own table scope, so
/// re-qualifying them with the joined table would produce wrong SQL.
pub(crate) fn qualify_bool_expr(expr: BoolExpr, table: &str) -> BoolExpr {
    match expr {
        BoolExpr::Filter(condition) => BoolExpr::Filter(condition.qualified(table)),
        BoolExpr::And(left, right) => BoolExpr::And(
            Box::new(qualify_bool_expr(*left, table)),
            Box::new(qualify_bool_expr(*right, table)),
        ),
        BoolExpr::Or(left, right) => BoolExpr::Or(
            Box::new(qualify_bool_expr(*left, table)),
            Box::new(qualify_bool_expr(*right, table)),
        ),
        BoolExpr::Not(inner) => BoolExpr::Not(Box::new(qualify_bool_expr(*inner, table))),
        other => other,
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

/// The SQL JOIN type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// `INNER JOIN`.
    Inner,
    /// `LEFT JOIN`.
    Left,
    /// `RIGHT JOIN`.
    Right,
    /// `FULL JOIN` (not supported by MySQL).
    Full,
    /// `CROSS JOIN` (no ON clause).
    Cross,
}

/// One equality key of a JOIN's `ON` clause.
#[derive(Debug, Clone)]
pub struct JoinKey {
    /// Table on the left-hand side of the equality — the FROM table (or the
    /// previously joined table for chained joins).
    pub left_table: String,
    /// Column on `left_table`.
    pub left_column: String,
    /// Column on the joined table.
    pub right_column: String,
}

/// A JOIN specification for SQL generation.
///
/// A composite key is represented as multiple [`JoinKey`]s joined with `AND`;
/// `CROSS JOIN` carries no keys.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct JoinSpec {
    /// JOIN type.
    pub kind: JoinKind,
    /// The table to join.
    pub table: String,
    /// Equality keys of the `ON` clause (empty for `CROSS JOIN`).
    pub keys: Vec<JoinKey>,
}

impl JoinSpec {
    /// Creates a join specification.
    ///
    /// Prefer [`QueryBuilder::join`](crate::query::QueryBuilder::join) /
    /// [`join_from`](crate::query::QueryBuilder::join_from), which resolve the
    /// left table; this constructor exists for internal/codegen use and tests.
    pub fn new(kind: JoinKind, table: impl Into<String>, keys: Vec<JoinKey>) -> Self {
        Self {
            kind,
            table: table.into(),
            keys,
        }
    }
}

impl JoinSpec {
    pub fn to_sql(&self) -> String {
        let join_kw = match self.kind {
            JoinKind::Inner => "INNER",
            JoinKind::Left => "LEFT",
            JoinKind::Right => "RIGHT",
            JoinKind::Full => "FULL",
            JoinKind::Cross => return format!("CROSS JOIN {}", self.table),
        };
        let on_clause = self
            .keys
            .iter()
            .map(|key| {
                format!(
                    "{}.{} = {}.{}",
                    key.left_table, key.left_column, self.table, key.right_column
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ");
        // A join without keys is a cross product; emitting `ON 1 = 1` keeps the
        // statement valid instead of producing a dangling `JOIN t ON`.
        let on_clause = if on_clause.is_empty() {
            "1 = 1".to_string()
        } else {
            on_clause
        };
        format!("{} JOIN {} ON {}", join_kw, self.table, on_clause)
    }
}

// Note: `HavingExpr`, `AggKind`, and `CompareOp` live in `having_expr.rs`.
// `GroupBy` and `HavingCondition` were removed as dead code (superseded by
// `Vec<String>` and `HavingExpr` respectively).
