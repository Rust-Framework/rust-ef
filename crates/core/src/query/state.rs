//! `QueryState` — accumulator for a single query's intent.
//!
//! Holds filter conditions, JOINs, GROUP BY, HAVING, ORDER BY, pagination,
//! includes, projections, window functions, CTEs, and set operations.
//! `to_sql_with` compiles the state into a SQL string using the provider's
//! placeholder syntax; `all_params` returns the bound parameter vector in
//! the order expected by the generated SQL (CTE params first, then WHERE/
//! HAVING, then set operation operand params).

use crate::provider::DbValue;

use super::ast::{BoolExpr, FilterCondition, IncludePath, JoinSpec, OrderBy};
use super::compile::{build_where_clauses, compile_bool_expr, PortablePlaceholderGenerator};
use super::cte::{CteSpec, SetOpSpec, SetOperator};
use super::having_expr::HavingExpr;
use super::window::WindowSpec;

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
    /// Set operations (UNION / INTERSECT / EXCEPT) appended after the main
    /// SELECT. Multiple operations chain left-to-right.
    pub set_operations: Vec<SetOpSpec>,
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
            set_operations: Vec::new(),
        }
    }

    pub(crate) fn append_bool_expr(&mut self, expr: BoolExpr) {
        self.where_expr = Some(match self.where_expr.take() {
            None => expr,
            Some(existing) => BoolExpr::And(Box::new(existing), Box::new(expr)),
        });
    }

    pub(crate) fn append_filter(&mut self, condition: FilterCondition) {
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

        // Pre-allocate a single buffer instead of chaining format!() calls
        // that each allocate a temporary String and copy it via push_str.
        let mut sql = String::with_capacity(256);
        sql.push_str(&select);
        sql.push_str(" FROM ");
        sql.push_str(&self.from);

        // JOINs
        for join in &self.joins {
            sql.push(' ');
            sql.push_str(&join.to_sql());
        }

        // Parameter index is shared across WHERE and HAVING so that
        // PostgreSQL's 1-indexed `$N` placeholders remain contiguous and
        // correctly ordered across both clauses. CTE parameters (if any)
        // occupy the leading slots, so the main query starts after them.
        let mut param_idx = 1usize + cte_param_count;

        // WHERE
        if let Some(ref expr) = self.where_expr {
            sql.push_str(" WHERE ");
            sql.push_str(&compile_bool_expr(expr, gen, &mut param_idx));
        } else if !self.filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&build_where_clauses(&self.filters, gen));
            // Advance `param_idx` past the legacy `filters` path so that
            // HAVING placeholders (PostgreSQL `$N`) continue from the
            // correct index. `build_where_clauses` always starts at index 1.
            param_idx += self.filters.iter().map(|f| f.param_count()).sum::<usize>();
        }

        // GROUP BY
        if !self.group_bys.is_empty() {
            sql.push_str(" GROUP BY ");
            sql.push_str(&self.group_bys.join(", "));
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
            sql.push_str(" HAVING ");
            sql.push_str(&compiled.join(" AND "));
        }

        // ORDER BY
        if !self.orderings.is_empty() {
            let ords: Vec<String> = self.orderings.iter().map(|o| o.to_sql()).collect();
            sql.push_str(" ORDER BY ");
            sql.push_str(&ords.join(", "));
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
            let has_recursive = self.ctes.iter().any(|c| c.is_recursive);
            for c in &self.ctes {
                let body = if !c.table.is_empty() {
                    // Typed mode: compile WHERE expression starting at the
                    // running index. `cte_idx` advances as placeholders are
                    // emitted; we then propagate it back to `running_idx`.
                    let mut cte_idx = running_idx;
                    let table = gen.quote_identifier(&c.table);
                    let anchor = match &c.where_expr {
                        Some(expr) => {
                            let where_sql = compile_bool_expr(expr, gen, &mut cte_idx);
                            format!("SELECT * FROM {} WHERE {}", table, where_sql)
                        }
                        None => format!("SELECT * FROM {}", table),
                    };
                    running_idx = cte_idx;
                    if c.is_recursive {
                        // Append recursive member:
                        // SELECT t.* FROM <table> t JOIN <name> ON t.<fk> = <name>.<pk>
                        let (fk, pk) = c
                            .recursive_link
                            .clone()
                            .expect("recursive CTE must have recursive_link");
                        let name = gen.quote_identifier(&c.name);
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
            let with_kw = if has_recursive {
                "WITH RECURSIVE"
            } else {
                "WITH"
            };
            // Prepend the CTE prefix to `sql` without allocating a new String
            // via format!("{} {} {}", ...).
            let joined = cte_parts.join(", ");
            let mut prefix = String::with_capacity(with_kw.len() + 1 + joined.len() + 1);
            prefix.push_str(with_kw);
            prefix.push(' ');
            prefix.push_str(&joined);
            prefix.push(' ');
            sql.insert_str(0, &prefix);
        }

        // Set operations (UNION / INTERSECT / EXCEPT) — appended after the
        // full SELECT (including CTE prefix). Operands are pre-compiled SQL
        // strings; their params are appended in all_params() after the main
        // query params. Per D5, operands should not contain ORDER BY / LIMIT.
        for op in &self.set_operations {
            let kw = match op.operator {
                SetOperator::Union => " UNION ",
                SetOperator::UnionAll => " UNION ALL ",
                SetOperator::Intersect => " INTERSECT ",
                SetOperator::Except => " EXCEPT ",
            };
            sql.push_str(kw);
            sql.push_str(&op.operand_sql);
        }

        sql
    }

    /// Returns all parameter values for execution: CTE parameters first
    /// (in declaration order), followed by WHERE/HAVING parameters, then
    /// set operation operand params.
    ///
    /// This ordering matches the placeholder order in the generated SQL,
    /// where CTE bodies appear before the main SELECT and set operations
    /// appear after.
    pub fn all_params(&self) -> Vec<DbValue> {
        let mut params = Vec::new();
        for cte in &self.ctes {
            params.extend(cte.params.clone());
        }
        params.extend(self.parameters.clone());
        for op in &self.set_operations {
            params.extend(op.operand_params.clone());
        }
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
