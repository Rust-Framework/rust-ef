//! `QueryState` — accumulator for a single query's intent.
//!
//! `QueryState` is the single source of truth for a query: FROM/JOIN, the
//! boolean WHERE tree, grouping/HAVING, ordering, pagination, includes, the
//! SELECT list ([`SelectList`]), window projections, CTEs and set operations.
//!
//! SQL compilation is single-pass: [`QueryState::try_compile_with`] walks the
//! state in SQL text order and emits placeholders **while collecting the bound
//! values**, so the SQL string and the parameter vector cannot drift apart.
//! [`to_sql_with`](Self::to_sql_with) / [`all_params`](Self::all_params) are
//! compatibility helpers for call sites that only need one half; built-in
//! terminals use the single-pass entry point.
//!
//! Parameter order is fixed by construction:
//! CTE bodies → SELECT-list projections → WHERE → HAVING → set-operation
//! operands.

use crate::error::{EFError, EFResult};
use crate::provider::{DbValue, ISqlGenerator};

use super::ast::{BoolExpr, IncludePath, JoinSpec, OrderBy};
use super::compile::{
    compile_bool_expr_into, emit_raw_fragment, validate_raw_fragment, PortablePlaceholderGenerator,
};
use super::cte::{CteSpec, SetOpSpec, SetOperator};
use super::expr::SelectItem;
use super::window::WindowSpec;

/// The SELECT list of a query.
///
/// One enum replaces the previous six independent flags/fields
/// (`is_count`, `is_exists`, `aggregate`, `aggregate_column`,
/// `projected_columns`, `projections`): a query has exactly one SELECT shape,
/// and the compiler matches on it instead of relying on a field-priority
/// convention.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub enum SelectList {
    /// `SELECT *` (the default for entity queries).
    #[default]
    Star,
    /// A plain runtime column list (`SELECT a, b`).
    Columns(Vec<String>),
    /// Typed projections (`SelectItem`): columns, aggregates, COALESCE/CASE,
    /// literals with aliases.
    Projections(Vec<SelectItem>),
    /// A single aggregate over one column (`SELECT SUM(amount)`), used by the
    /// `sum`/`avg`/`min`/`max` terminals.
    Aggregate {
        /// SQL aggregate keyword (`SUM`, `AVG`, `MIN`, `MAX`).
        func: String,
        /// Aggregated column; `None` means `*` (`COUNT(*)`).
        column: Option<String>,
    },
    /// `SELECT COUNT(*)` (COUNT terminal).
    Count,
    /// `SELECT 1` (EXISTS terminal).
    Exists,
}

impl SelectList {
    /// Returns the typed projections when this is [`SelectList::Projections`].
    pub fn projections(&self) -> &[SelectItem] {
        match self {
            SelectList::Projections(items) => items,
            _ => &[],
        }
    }

    /// Returns the plain column list when this is [`SelectList::Columns`].
    pub fn columns(&self) -> Option<&[String]> {
        match self {
            SelectList::Columns(columns) => Some(columns),
            _ => None,
        }
    }

    /// Whether this is a COUNT SELECT.
    pub fn is_count(&self) -> bool {
        matches!(self, SelectList::Count)
    }

    /// Whether this is an EXISTS SELECT.
    pub fn is_exists(&self) -> bool {
        matches!(self, SelectList::Exists)
    }

    /// The aggregate function/column pair, when this is an aggregate SELECT.
    pub fn aggregate(&self) -> Option<(&str, Option<&str>)> {
        match self {
            SelectList::Aggregate { func, column } => Some((func.as_str(), column.as_deref())),
            _ => None,
        }
    }
}

/// Accumulated intent for a single query.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QueryState {
    /// FROM table / subquery segment.
    pub from: String,
    /// JOIN specifications.
    pub joins: Vec<JoinSpec>,
    /// GROUP BY columns.
    pub group_bys: Vec<String>,
    /// HAVING conditions stored as AST nodes; lowered to
    /// [`BoolExpr::Compare`] and compiled by the same compiler as WHERE.
    pub havings: Vec<BoolExpr>,
    /// ORDER BY clauses.
    pub orderings: Vec<OrderBy>,
    /// OFFSET (Skip).
    pub offset: Option<usize>,
    /// LIMIT (Take).
    pub limit: Option<usize>,
    /// Include navigation paths.
    pub includes: Vec<IncludePath>,
    /// The SELECT list (single source of truth for the query shape).
    select: SelectList,
    /// Boolean WHERE expression (the only filter representation).
    pub where_expr: Option<BoolExpr>,
    /// Whether to emit `SELECT DISTINCT`.
    pub distinct: bool,
    /// Window function projections. Emitted in the SELECT list as
    /// `func(col) OVER (...) AS alias`.
    pub windows: Vec<WindowSpec>,
    /// CTE definitions, emitted as a `WITH` prefix.
    pub ctes: Vec<CteSpec>,
    /// Set operations (UNION / INTERSECT / EXCEPT) appended after the main
    /// SELECT. Multiple operations chain left-to-right.
    pub set_operations: Vec<SetOpSpec>,
    /// Deferred builder-time validation error (e.g. `from_cte` after a JOIN,
    /// or a `filter_joined` predicate containing a subquery). Terminals report
    /// it before executing; plain `to_sql` only debug-asserts.
    validation_error: Option<String>,
    /// Set when `from_cte` runs after join clauses were added: a JOIN resolves
    /// its left table from the FROM active at join time, so the query would
    /// reference the root table while FROM points at a CTE.
    from_changed_after_join: bool,
}

impl QueryState {
    /// Creates an empty state for `from`.
    pub fn new(from: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            joins: Vec::new(),
            group_bys: Vec::new(),
            havings: Vec::new(),
            orderings: Vec::new(),
            offset: None,
            limit: None,
            includes: Vec::new(),
            select: SelectList::Star,
            where_expr: None,
            distinct: false,
            windows: Vec::new(),
            ctes: Vec::new(),
            set_operations: Vec::new(),
            validation_error: None,
            from_changed_after_join: false,
        }
    }

    // ------------------------------------------------------------------
    // SELECT list accessors
    // ------------------------------------------------------------------

    /// Returns the SELECT list.
    pub fn select_list(&self) -> &SelectList {
        &self.select
    }

    /// Returns the typed projections (empty unless the SELECT is a projection).
    pub fn projections(&self) -> &[SelectItem] {
        self.select.projections()
    }

    /// Returns the plain projected columns, if the SELECT is a column list.
    pub fn projected_columns(&self) -> Option<&[String]> {
        self.select.columns()
    }

    /// Whether this query selects `COUNT(*)`.
    pub fn is_count(&self) -> bool {
        self.select.is_count()
    }

    /// Whether this query selects `1` (EXISTS).
    pub fn is_exists(&self) -> bool {
        self.select.is_exists()
    }

    /// The aggregate function/column pair for aggregate terminals.
    pub fn aggregate(&self) -> Option<(&str, Option<&str>)> {
        self.select.aggregate()
    }

    /// Replaces the SELECT list with typed projections.
    pub fn set_projections(&mut self, projections: Vec<SelectItem>) {
        self.select = SelectList::Projections(projections);
    }

    /// Appends one typed projection, promoting a plain column list first.
    pub fn push_projection(&mut self, item: SelectItem) {
        let existing = std::mem::take(&mut self.select);
        let mut items = match existing {
            SelectList::Projections(items) => items,
            SelectList::Columns(columns) => columns.into_iter().map(SelectItem::column).collect(),
            _ => Vec::new(),
        };
        items.push(item);
        self.select = SelectList::Projections(items);
    }

    /// Appends typed projections, promoting a plain column list first.
    pub fn extend_projections(&mut self, items: impl IntoIterator<Item = SelectItem>) {
        let existing = std::mem::take(&mut self.select);
        let mut projections = match existing {
            SelectList::Projections(items) => items,
            SelectList::Columns(columns) => columns.into_iter().map(SelectItem::column).collect(),
            _ => Vec::new(),
        };
        projections.extend(items);
        self.select = SelectList::Projections(projections);
    }

    /// Takes the plain projected columns out of the SELECT list, if any.
    pub fn take_projected_columns(&mut self) -> Option<Vec<String>> {
        let existing = std::mem::take(&mut self.select);
        match existing {
            SelectList::Columns(columns) => Some(columns),
            other => {
                self.select = other;
                None
            }
        }
    }

    /// Replaces the SELECT list with a plain column list.
    pub fn set_projected_columns(&mut self, columns: Vec<String>) {
        self.select = SelectList::Columns(columns);
    }

    /// Marks the query as `SELECT COUNT(*)`.
    pub fn set_count(&mut self) {
        self.select = SelectList::Count;
    }

    /// Marks the query as `SELECT 1` (EXISTS).
    pub fn set_exists(&mut self) {
        self.select = SelectList::Exists;
    }

    /// Marks the query as a single aggregate over `column`.
    pub fn set_aggregate(&mut self, func: impl Into<String>, column: Option<String>) {
        self.select = SelectList::Aggregate {
            func: func.into(),
            column,
        };
    }

    /// Resets the SELECT list to `SELECT *`.
    pub fn set_select_star(&mut self) {
        self.select = SelectList::Star;
    }

    // ------------------------------------------------------------------
    // Deferred validation
    // ------------------------------------------------------------------

    /// Records a builder-time validation error to be reported by the terminal.
    pub fn set_validation_error(&mut self, message: impl Into<String>) {
        self.validation_error = Some(message.into());
    }

    /// Returns the deferred validation error, if any.
    pub fn validation_error(&self) -> Option<&str> {
        self.validation_error.as_deref()
    }

    /// Records that `from_cte` was called after at least one JOIN.
    pub fn mark_from_changed_after_join(&mut self) {
        self.from_changed_after_join = true;
    }

    pub(crate) fn append_bool_expr(&mut self, expr: BoolExpr) {
        self.where_expr = Some(match self.where_expr.take() {
            None => expr,
            Some(existing) => BoolExpr::And(Box::new(existing), Box::new(expr)),
        });
    }

    // ------------------------------------------------------------------
    // Compilation
    // ------------------------------------------------------------------

    /// Validates the state, then compiles SQL and parameters in one pass.
    ///
    /// This is the entry point used by every terminal. `EXISTS`/`IN` subquery
    /// metadata must already be resolved (see
    /// [`compile_sql_with`](Self::compile_sql_with)).
    pub fn try_compile_with(&self, gen: &dyn ISqlGenerator) -> EFResult<(String, Vec<DbValue>)> {
        self.validate()?;
        Ok(self.compile_unchecked(gen))
    }

    /// Validates the query for execution.
    pub fn validate(&self) -> EFResult<()> {
        if let Some(message) = &self.validation_error {
            return Err(EFError::configuration(message.clone()));
        }
        if self.from_changed_after_join {
            return Err(EFError::configuration(
                "`from`/`from_cte` must be called before `join`/`join_from`: a JOIN \
                 resolves its left table from the FROM active when it is added"
                    .to_string(),
            ));
        }
        if self.distinct {
            match &self.select {
                SelectList::Count => {
                    return Err(EFError::configuration(
                        "`COUNT(*)` cannot be combined with `DISTINCT`: project a column \
                         and use `SelectItem::count_distinct(column)`"
                            .to_string(),
                    ));
                }
                SelectList::Aggregate { column: None, .. } => {
                    return Err(EFError::configuration(
                        "an aggregate over `*` cannot be combined with `DISTINCT`".to_string(),
                    ));
                }
                _ => {}
            }
        }
        match &self.select {
            SelectList::Projections(items) => {
                if items.is_empty() {
                    return Err(EFError::configuration(
                        "a projection query needs at least one SelectItem".to_string(),
                    ));
                }
                let mut aliases = std::collections::HashSet::new();
                for item in items {
                    if !aliases.insert(item.alias_name()) {
                        return Err(EFError::configuration(format!(
                            "duplicate projection alias `{}`: give one of the same-named \
                             columns an explicit `.alias(..)`",
                            item.alias_name()
                        )));
                    }
                }
            }
            SelectList::Columns(columns) if columns.is_empty() => {
                return Err(EFError::configuration(
                    "a column projection needs at least one column".to_string(),
                ));
            }
            SelectList::Star if !self.group_bys.is_empty() || !self.havings.is_empty() => {
                return Err(EFError::configuration(
                    "a grouped query needs an explicit projection: use \
                     select_items(...) (or linq! `select` with aggregate items) plus \
                     to_list_rows()/to_list_typed_*"
                        .to_string(),
                ));
            }
            SelectList::Aggregate { .. } | SelectList::Count | SelectList::Exists
                if !self.group_bys.is_empty() =>
            {
                return Err(EFError::configuration(
                    "a single-value aggregate terminal cannot be combined with GROUP BY \
                     (one row per group); use select_items(...) plus to_list_rows()"
                        .to_string(),
                ));
            }
            _ => {}
        }
        if matches!(
            self.select,
            SelectList::Aggregate { .. } | SelectList::Count
        ) {
            if !self.set_operations.is_empty() {
                return Err(EFError::configuration(
                    "a single-value aggregate terminal cannot be combined with set \
                     operations (UNION/INTERSECT/EXCEPT)"
                        .to_string(),
                ));
            }
            if self.offset.is_some() || self.limit.is_some() {
                return Err(EFError::configuration(
                    "a single-value aggregate terminal cannot be combined with \
                     skip/take: count a projection or wrap the paginated query instead"
                        .to_string(),
                ));
            }
            if !self.orderings.is_empty() {
                return Err(EFError::configuration(
                    "ORDER BY has no effect on a single-value aggregate terminal".to_string(),
                ));
            }
            if !self.windows.is_empty() {
                return Err(EFError::configuration(
                    "window projections require an explicit projection, not a \
                     single-value aggregate terminal"
                        .to_string(),
                ));
            }
        }
        if self.select.is_exists() {
            if !self.set_operations.is_empty() {
                return Err(EFError::configuration(
                    "EXISTS cannot be combined with set operations".to_string(),
                ));
            }
            if !self.windows.is_empty() {
                return Err(EFError::configuration(
                    "window projections cannot be combined with EXISTS".to_string(),
                ));
            }
        }
        self.validate_raw_fragments()?;
        Ok(())
    }

    /// Validates pre-compiled raw fragments (raw CTEs / set-operation operands)
    /// against their bound values so a placeholder/parameter mismatch is a typed
    /// error instead of a runtime "wrong number of bindings".
    fn validate_raw_fragments(&self) -> EFResult<()> {
        for cte in self.ctes.iter().filter(|cte| cte.table.is_empty()) {
            validate_raw_fragment(&cte.sql, &cte.params, "raw CTE")?;
        }
        for op in &self.set_operations {
            validate_raw_fragment(&op.operand_sql, &op.operand_params, "set-operation operand")?;
        }
        Ok(())
    }

    /// Validates the state for a bulk `UPDATE` / `DELETE`.
    ///
    /// DML only supports `FROM` + `WHERE` (a correlated subquery predicate is
    /// fine because it is part of `where_expr`). Joins, CTEs, grouping, HAVING,
    /// ordering, pagination, set operations, includes, windows, DISTINCT and
    /// projection shapes are rejected instead of being silently dropped from
    /// the generated statement.
    pub fn validate_for_dml(&self) -> EFResult<()> {
        self.validate()?;
        let unsupported = [
            ("JOIN", !self.joins.is_empty()),
            ("CTE", !self.ctes.is_empty()),
            ("GROUP BY", !self.group_bys.is_empty()),
            ("HAVING", !self.havings.is_empty()),
            ("ORDER BY", !self.orderings.is_empty()),
            ("skip", self.offset.is_some()),
            ("take", self.limit.is_some()),
            ("set operations", !self.set_operations.is_empty()),
            ("DISTINCT", self.distinct),
            ("includes", !self.includes.is_empty()),
            ("window projections", !self.windows.is_empty()),
            ("a projection", !matches!(self.select, SelectList::Star)),
        ];
        for (clause, present) in unsupported {
            if present {
                return Err(EFError::configuration(format!(
                    "{clause} is not supported in bulk UPDATE/DELETE; filter the \
                     entity query or use raw SQL with IRow"
                )));
            }
        }
        Ok(())
    }

    /// Compiles the state without validation (compatibility helpers and tests).
    fn compile_unchecked(&self, gen: &dyn ISqlGenerator) -> (String, Vec<DbValue>) {
        let mut params: Vec<DbValue> = Vec::new();
        let mut param_idx = 1usize;

        // 1) CTE bodies occupy the first parameter slots (SQL text order).
        let cte_prefix = self.compile_cte_prefix(gen, &mut param_idx, &mut params);

        // 2) The SELECT list may bind parameters (projections/literals/CASE).
        let select = self.compile_select(gen, &mut param_idx, &mut params);

        let mut sql = String::with_capacity(256);
        sql.push_str(&select);
        sql.push_str(" FROM ");
        // `from` is a CTE reference exactly when it names a CTE declared in
        // this state. Deriving that from the CTE list (instead of a separate
        // flag) keeps the quoted `WITH` name and its `FROM` reference in sync
        // even when a caller assigns `from` directly.
        if self.ctes.iter().any(|cte| cte.name == self.from) {
            sql.push_str(&gen.quote_identifier(&self.from));
        } else {
            sql.push_str(&self.from);
        }

        for join in &self.joins {
            sql.push(' ');
            sql.push_str(&join.to_sql_with(gen));
        }

        if let Some(expr) = &self.where_expr {
            sql.push_str(" WHERE ");
            sql.push_str(&compile_bool_expr_into(
                expr,
                gen,
                &mut param_idx,
                &mut params,
            ));
        }

        if !self.group_bys.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&self.group_bys.join(", "));
        }

        if !self.havings.is_empty() {
            let compiled: Vec<String> = self
                .havings
                .iter()
                .map(|having| compile_bool_expr_into(having, gen, &mut param_idx, &mut params))
                .collect();
            sql.push_str(" HAVING ");
            if compiled.len() == 1 {
                sql.push_str(&compiled[0]);
            } else {
                // A HAVING fragment may itself be an OR tree: joining fragments
                // without parentheses would re-associate `a AND (b) OR (c)`.
                let wrapped: Vec<String> = compiled
                    .iter()
                    .map(|fragment| format!("({fragment})"))
                    .collect();
                sql.push_str(&wrapped.join(" AND "));
            }
        }

        // 2b) Set operations belong to the query body: ORDER BY / LIMIT / OFFSET
        //     after a set operation apply to the combined result, so append the
        //     operands before them.
        for op in &self.set_operations {
            let keyword = match op.operator {
                SetOperator::Union => " UNION ",
                SetOperator::UnionAll => " UNION ALL ",
                SetOperator::Intersect => " INTERSECT ",
                SetOperator::Except => " EXCEPT ",
            };
            sql.push_str(keyword);
            sql.push_str(&emit_raw_fragment(
                &op.operand_sql,
                &op.operand_params,
                gen,
                &mut param_idx,
                &mut params,
                "set-operation operand",
            ));
        }

        if !self.orderings.is_empty() {
            let orderings: Vec<String> = self.orderings.iter().map(|o| o.to_sql()).collect();
            sql.push_str(" ORDER BY ");
            sql.push_str(&orderings.join(", "));
        }

        let pagination = gen.pagination(self.offset, self.limit);
        if !pagination.is_empty() {
            sql.push(' ');
            sql.push_str(&pagination);
        }

        // 4) Prepend the WITH prefix (its params were emitted first).
        if let Some(prefix) = cte_prefix {
            sql.insert_str(0, &prefix);
        }

        (sql, params)
    }

    /// Compiles the `WITH ...` prefix, collecting CTE parameters.
    fn compile_cte_prefix(
        &self,
        gen: &dyn ISqlGenerator,
        param_idx: &mut usize,
        params: &mut Vec<DbValue>,
    ) -> Option<String> {
        if self.ctes.is_empty() {
            return None;
        }
        let has_recursive = self.ctes.iter().any(|cte| cte.is_recursive);
        let mut parts = Vec::with_capacity(self.ctes.len());
        for cte in &self.ctes {
            let body = if !cte.table.is_empty() {
                let table = gen.quote_identifier(&cte.table);
                let anchor = match &cte.where_expr {
                    Some(expr) => {
                        let where_sql = compile_bool_expr_into(expr, gen, param_idx, params);
                        format!("SELECT * FROM {} WHERE {}", table, where_sql)
                    }
                    None => format!("SELECT * FROM {}", table),
                };
                if cte.is_recursive {
                    let (fk, pk) = cte
                        .recursive_link
                        .clone()
                        .expect("recursive CTE must have recursive_link");
                    let name = gen.quote_identifier(&cte.name);
                    let fk = gen.quote_identifier(&fk);
                    let pk = gen.quote_identifier(&pk);
                    format!(
                        "{} UNION ALL SELECT t.* FROM {} t JOIN {} ON t.{} = {}.{}",
                        anchor, table, name, fk, name, pk
                    )
                } else {
                    anchor
                }
            } else {
                emit_raw_fragment(&cte.sql, &cte.params, gen, param_idx, params, "raw CTE")
            };

            // Framework-generated identifiers (CTE names/columns) are quoted;
            // caller-supplied table names in FROM remain code constants emitted
            // as provided (documented identifier policy).
            let cte_name = gen.quote_identifier(&cte.name);
            let part = if cte.columns.is_empty() {
                format!("{} AS ({})", cte_name, body)
            } else {
                let columns = cte
                    .columns
                    .iter()
                    .map(|column| gen.quote_identifier(column))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} ({}) AS ({})", cte_name, columns, body)
            };
            parts.push(part);
        }
        let with_kw = if has_recursive {
            "WITH RECURSIVE"
        } else {
            "WITH"
        };
        Some(format!("{} {} ", with_kw, parts.join(", ")))
    }

    /// Compiles the SELECT list, collecting projection parameters.
    fn compile_select(
        &self,
        gen: &dyn ISqlGenerator,
        param_idx: &mut usize,
        params: &mut Vec<DbValue>,
    ) -> String {
        let distinct_kw = if self.distinct { "DISTINCT " } else { "" };
        match &self.select {
            SelectList::Count => "SELECT COUNT(*)".to_string(),
            SelectList::Exists => "SELECT 1".to_string(),
            SelectList::Aggregate { func, column } => {
                let column = column.as_deref().unwrap_or("*");
                if self.distinct {
                    format!("SELECT {}(DISTINCT {})", func, column)
                } else {
                    format!("SELECT {}({})", func, column)
                }
            }
            SelectList::Projections(items) => {
                let mut parts: Vec<String> = items
                    .iter()
                    .map(|item| item.compile(gen, param_idx, params))
                    .collect();
                for window in &self.windows {
                    parts.push(window.to_sql(gen));
                }
                format!("SELECT {}{}", distinct_kw, parts.join(", "))
            }
            SelectList::Columns(columns) => {
                let mut parts = columns.clone();
                for window in &self.windows {
                    parts.push(window.to_sql(gen));
                }
                format!("SELECT {}{}", distinct_kw, parts.join(", "))
            }
            SelectList::Star => {
                if self.windows.is_empty() {
                    format!("SELECT {}*", distinct_kw)
                } else {
                    let mut parts: Vec<String> = vec![format!("{}*", distinct_kw)];
                    for window in &self.windows {
                        parts.push(window.to_sql(gen));
                    }
                    format!("SELECT {}", parts.join(", "))
                }
            }
        }
    }

    /// Compiles SQL with the provider's placeholder syntax (unchecked).
    ///
    /// Prefer [`try_compile_with`](Self::try_compile_with) so an invalid query
    /// is a typed error rather than a debug assertion.
    pub fn to_sql_with(&self, gen: &dyn ISqlGenerator) -> String {
        // Unchecked by design: SQL-shape tests and logging may inspect a state
        // that is not executable. Terminals call `try_compile_with`.
        self.compile_unchecked(gen).0
    }

    /// Compiles SQL with `?` placeholders (SQLite/MySQL style).
    pub fn to_sql(&self) -> String {
        self.to_sql_with(&PortablePlaceholderGenerator)
    }

    /// Returns all execution parameters in SQL text order (single-pass
    /// compilation with the portable generator).
    pub fn all_params(&self) -> Vec<DbValue> {
        self.compile_unchecked(&PortablePlaceholderGenerator).1
    }

    /// Resolves correlated subqueries for `T`, validates, and returns SQL +
    /// full parameter vector in text order.
    pub fn compile_sql_with<T: crate::entity::IEntityType>(
        &self,
        gen: &dyn ISqlGenerator,
    ) -> EFResult<(String, Vec<DbValue>)> {
        let mut state = self.clone();
        super::compile::resolve_state_subqueries::<T>(&mut state);
        state.try_compile_with(gen)
    }

    /// Returns the values bound by the WHERE expression, in text order.
    pub fn where_params(&self) -> Vec<DbValue> {
        let Some(expr) = &self.where_expr else {
            return Vec::new();
        };
        let mut param_idx = 1usize;
        let mut params = Vec::new();
        let _ = compile_bool_expr_into(
            expr,
            &PortablePlaceholderGenerator,
            &mut param_idx,
            &mut params,
        );
        params
    }
}
