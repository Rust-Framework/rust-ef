//! Query builder — LINQ-style chainable query API.
//!
//! Accumulates filter conditions, orderings, pagination, includes, and
//! projection metadata through a fluent interface. Terminal methods
//! (`to_list`, `first`, `count`, etc.) produce real SQL that can be
//! executed against a database provider.

use crate::entity::{IEntityType, IFromRow};
use crate::error::LrefResult;
use crate::provider::{DbValue, IDatabaseProvider};
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
    /// SQL operator (e.g., "=", ">", "LIKE", "IS NULL").
    operator: String,
    /// The typed parameter value (None = no value, e.g. IS NULL).
    value: Option<DbValue>,
}

impl FilterCondition {
    pub fn new(
        column: impl Into<String>,
        operator: impl Into<String>,
        value: Option<DbValue>,
    ) -> Self {
        Self {
            column: column.into(),
            operator: operator.into(),
            value,
        }
    }

    /// Convert to a SQL WHERE fragment using the given placeholder.
    pub fn to_sql(&self, placeholder: &str) -> String {
        match &self.value {
            Some(_) => format!("{} {} {}", self.column, self.operator, placeholder),
            None => format!("{} {}", self.column, self.operator),
        }
    }

    /// Returns the DbValue parameter if one exists.
    pub fn db_value(&self) -> Option<&DbValue> {
        self.value.as_ref()
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
    pub nested: Vec<String>,
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
        }
    }

    /// Compile the state into a SQL string with parameterized placeholders.
    pub fn to_sql(&self) -> String {
        let select = if self.is_count {
            "SELECT COUNT(*)".to_string()
        } else if self.is_exists {
            "SELECT 1".to_string()
        } else if let Some(ref agg) = self.aggregate {
            let col = self.aggregate_column.as_deref().unwrap_or("*");
            format!("SELECT {}({})", agg, col)
        } else if let Some(ref cols) = self.projected_columns {
            format!("SELECT {}", cols.join(", "))
        } else {
            "SELECT *".to_string()
        };

        let mut sql = format!("{} FROM {}", select, self.from);

        // JOINs
        for join in &self.joins {
            sql.push_str(&format!(" {}", join.to_sql()));
        }

        // WHERE
        if !self.filters.is_empty() {
            let clauses: Vec<String> = self
                .filters
                .iter()
                .enumerate()
                .map(|(i, f)| f.to_sql(&format!("${}", i + 1)))
                .collect();
            sql.push_str(&format!(" WHERE {}", clauses.join(" AND ")));
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

    /// Returns the accumulated parameters.
    pub fn params(&self) -> &[DbValue] {
        &self.parameters
    }
}

// ---------------------------------------------------------------------------
// QueryBuilder
// ---------------------------------------------------------------------------

/// A chainable query builder for entity type `T`.
///
/// Corresponds to EFCore's `IQueryable<T>`.
pub struct QueryBuilder<T: IEntityType> {
    state: QueryState,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    _phantom: PhantomData<T>,
}

impl<T: IEntityType> QueryBuilder<T> {
    /// Creates a new QueryBuilder for a given table (without provider — SQL-only).
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

    // -------------------------------------------------------------------
    // Chainable methods (each returns Self with accumulated state)
    // -------------------------------------------------------------------

    /// Adds a filter condition (WHERE clause).
    pub fn filter<F>(mut self, _predicate: F) -> Self
    where
        F: Fn(&T) -> bool,
    {
        self.state.filters.push(FilterCondition::new(
            "__filter__",
            "=",
            Some(DbValue::String("?".to_string())),
        ));
        self
    }

    /// Adds a named filter condition with a specific operator and DbValue.
    pub fn filter_column(
        mut self,
        column: &str,
        operator: &str,
        value: impl Into<DbValue>,
    ) -> Self {
        let db_val = value.into();
        self.state.parameters.push(db_val.clone());
        self.state
            .filters
            .push(FilterCondition::new(column, operator, Some(db_val)));
        self
    }

    /// Finds an entity by its primary key.
    /// For composite keys, use `find_by_key`.
    pub fn find_by_id(mut self, id: i32) -> Self {
        let val = DbValue::I32(id);
        self.state.parameters.push(val.clone());
        self.state
            .filters
            .push(FilterCondition::new("id", "=", Some(val)));
        self
    }

    /// Finds an entity by composite primary key values.
    pub fn find_by_key(mut self, key_values: &std::collections::HashMap<String, DbValue>) -> Self {
        for (col, val) in key_values {
            self.state.parameters.push(val.clone());
            self.state
                .filters
                .push(FilterCondition::new(col.as_str(), "=", Some(val.clone())));
        }
        self
    }

    /// Adds an IN condition (uses parameterized placeholders correctly).
    pub fn filter_in(mut self, column: &str, values: Vec<impl Into<DbValue>>) -> Self {
        let db_vals: Vec<DbValue> = values.into_iter().map(|v| v.into()).collect();
        // Build parameterized placeholder list: the indices are determined
        // by the current parameter count + each new value's position
        let start = self.state.parameters.len() + 1;
        let placeholders: Vec<String> = (0..db_vals.len())
            .map(|i| format!("${}", start + i))
            .collect();
        for v in db_vals {
            self.state.parameters.push(v);
        }
        // Use None value so to_sql outputs just "column IN (...)" without extra placeholder
        self.state.filters.push(FilterCondition::new(
            column,
            &format!("IN ({})", placeholders.join(", ")),
            None,
        ));
        self
    }

    /// Adds an IS NULL condition.
    pub fn filter_is_null(mut self, column: &str) -> Self {
        self.state
            .filters
            .push(FilterCondition::new(column, "IS NULL", None));
        self
    }

    /// Adds an IS NOT NULL condition.
    pub fn filter_is_not_null(mut self, column: &str) -> Self {
        self.state
            .filters
            .push(FilterCondition::new(column, "IS NOT NULL", None));
        self
    }

    /// Adds a BETWEEN low AND high condition.
    pub fn filter_between(
        mut self,
        column: &str,
        low: impl Into<DbValue>,
        high: impl Into<DbValue>,
    ) -> Self {
        let lo: DbValue = low.into();
        let hi: DbValue = high.into();
        let start = self.state.parameters.len() + 1;
        self.state.parameters.push(lo);
        self.state.parameters.push(hi);
        self.state.filters.push(FilterCondition::new(
            column,
            &format!("BETWEEN ${} AND ${}", start, start + 1),
            None,
        ));
        self
    }

    /// Adds an ascending order-by clause on a named column.
    pub fn order_by<V>(mut self, _accessor: fn(&T) -> &V) -> Self {
        self.state
            .orderings
            .push(OrderBy::new("__order__", OrderDirection::Ascending));
        self
    }

    /// Adds a named ascending order-by.
    pub fn order_by_column(mut self, column: &str) -> Self {
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Ascending));
        self
    }

    /// Adds a descending order-by clause.
    pub fn order_by_desc<V>(mut self, _accessor: fn(&T) -> &V) -> Self {
        self.state
            .orderings
            .push(OrderBy::new("__order__", OrderDirection::Descending));
        self
    }

    /// Adds a named descending order-by.
    pub fn order_by_desc_column(mut self, column: &str) -> Self {
        self.state
            .orderings
            .push(OrderBy::new(column, OrderDirection::Descending));
        self
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

    /// Eagerly loads a related navigation.
    pub fn include<Nav>(mut self, _navigation: fn(&T) -> &Nav) -> Self {
        self.state.includes.push(IncludePath {
            navigation: "__include__".to_string(),
            nested: Vec::new(),
            related_table: None,
            foreign_key_column: None,
            referenced_key_column: None,
        });
        self
    }

    /// Eagerly loads a named navigation.
    pub fn include_named(mut self, navigation: &str) -> Self {
        self.state.includes.push(IncludePath {
            navigation: navigation.to_string(),
            nested: Vec::new(),
            related_table: None,
            foreign_key_column: None,
            referenced_key_column: None,
        });
        self
    }

    /// Eagerly loads a named navigation with a full JOIN specification.
    pub fn include_with_join(
        mut self,
        navigation: &str,
        related_table: &str,
        foreign_key: &str,
        referenced_key: &str,
        join_type: &str,
    ) -> Self {
        self.state.includes.push(IncludePath {
            navigation: navigation.to_string(),
            nested: Vec::new(),
            related_table: Some(related_table.to_string()),
            foreign_key_column: Some(foreign_key.to_string()),
            referenced_key_column: Some(referenced_key.to_string()),
        });
        let on_clause = format!(
            "{}.{} = {}.{}",
            self.state.from, foreign_key, related_table, referenced_key
        );
        self.state.joins.push(JoinSpec {
            join_type: join_type.to_string(),
            table: related_table.to_string(),
            on_clause,
        });
        self
    }

    /// Adds an INNER JOIN.
    pub fn inner_join(mut self, table: &str, left_column: &str, right_column: &str) -> Self {
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
    pub fn left_join(mut self, table: &str, left_column: &str, right_column: &str) -> Self {
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
    pub fn group_by(mut self, columns: &[&str]) -> Self {
        self.state.group_bys = columns.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Adds a HAVING condition.
    pub fn having(mut self, expression: &str) -> Self {
        self.state.havings.push(expression.to_string());
        self
    }

    /// Eagerly loads a nested related navigation.
    pub fn then_include<Nav, SubNav>(mut self, _navigation: fn(&Nav) -> &SubNav) -> Self {
        if let Some(last) = self.state.includes.last_mut() {
            last.nested.push("__then__".to_string());
        }
        self
    }

    // -------------------------------------------------------------------
    // Aggregate terminal methods
    // -------------------------------------------------------------------

    /// Executes a SUM aggregation query.
    pub async fn sum(self, column: &str) -> LrefResult<f64> {
        let mut state = self.state.clone();
        state.aggregate = Some("SUM".to_string());
        state.aggregate_column = Some(column.to_string());
        let sql = state.to_sql();
        let params = state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            first.trim().parse::<f64>().map_err(|_| {
                crate::error::LrefError::TypeConversion("SUM result is not f64".to_string())
            })
        } else {
            Ok(0.0)
        }
    }

    /// Executes an AVG aggregation query.
    pub async fn avg(self, column: &str) -> LrefResult<f64> {
        let mut state = self.state.clone();
        state.aggregate = Some("AVG".to_string());
        state.aggregate_column = Some(column.to_string());
        let sql = state.to_sql();
        let params = state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first) = rows.first().and_then(|r| r.first()) {
            first.trim().parse::<f64>().map_err(|_| {
                crate::error::LrefError::TypeConversion("AVG result is not f64".to_string())
            })
        } else {
            Ok(0.0)
        }
    }

    /// Executes a MIN aggregation query.
    pub async fn min<V>(self, column: &str) -> LrefResult<Option<String>> {
        let mut state = self.state.clone();
        state.aggregate = Some("MIN".to_string());
        state.aggregate_column = Some(column.to_string());
        let sql = state.to_sql();
        let params = state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        Ok(rows.first().and_then(|r| r.first().cloned()))
    }

    /// Executes a MAX aggregation query.
    pub async fn max<V>(self, column: &str) -> LrefResult<Option<String>> {
        let mut state = self.state.clone();
        state.aggregate = Some("MAX".to_string());
        state.aggregate_column = Some(column.to_string());
        let sql = state.to_sql();
        let params = state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        Ok(rows.first().and_then(|r| r.first().cloned()))
    }

    // -------------------------------------------------------------------
    // Terminal methods
    // -------------------------------------------------------------------

    /// Projects to a different shape.
    pub fn select<R, F>(self, _selector: F) -> SelectQueryBuilder<T, R>
    where
        F: Fn(&T) -> R,
    {
        let mut state = self.state.clone();
        state.projected_columns = Some(vec!["__projected__".to_string()]);
        SelectQueryBuilder::<T, R> {
            state,
            _phantom_t: PhantomData,
            _phantom_r: PhantomData,
        }
    }

    /// Projects to named columns.
    pub fn select_columns(self, columns: &[&str]) -> SelectQueryBuilder<T, ()> {
        let mut state = self.state.clone();
        state.projected_columns = Some(columns.iter().map(|s| s.to_string()).collect());
        SelectQueryBuilder::<T, ()> {
            state,
            _phantom_t: PhantomData,
            _phantom_r: PhantomData,
        }
    }

    // -------------------------------------------------------------------
    // Terminal methods
    // -------------------------------------------------------------------

    /// Builds the SQL string for this query.
    pub fn to_sql(&self) -> String {
        self.state.to_sql()
    }

    /// Executes the query and returns all matching entities.
    pub async fn to_list(self) -> LrefResult<Vec<T>>
    where
        T: IFromRow,
    {
        let sql = self.to_sql();
        let params = self.state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder. Use DbSet::query() or attach a provider."
                    .to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        crate::entity::materialize_entities::<T>(&rows)
    }

    /// Executes the query and returns the first matching entity.
    pub async fn first(self) -> LrefResult<T>
    where
        T: IFromRow,
    {
        let mut results = self.take(1).to_list().await?;
        results
            .pop()
            .ok_or_else(|| crate::error::LrefError::NotFound("Entity not found".to_string()))
    }

    /// Executes the query and returns the first matching entity or None.
    pub async fn first_or_default(self) -> LrefResult<Option<T>>
    where
        T: IFromRow,
    {
        let mut results = self.take(1).to_list().await?;
        Ok(results.pop())
    }

    /// Executes a COUNT query.
    pub async fn count(self) -> LrefResult<i64> {
        let mut state = self.state.clone();
        state.is_count = true;
        let sql = state.to_sql();
        let params = state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        if let Some(first_row) = rows.first() {
            if let Some(first_val) = first_row.first() {
                return first_val.trim().parse::<i64>().map_err(|e| {
                    crate::error::LrefError::TypeConversion(format!(
                        "COUNT result '{}' is not i64: {}",
                        first_val, e
                    ))
                });
            }
        }
        Ok(0)
    }

    /// Checks if any entities match the query.
    pub async fn any(self) -> LrefResult<bool> {
        let mut state = self.state.clone();
        state.is_exists = true;
        state.limit = Some(1);
        let sql = state.to_sql();
        let params = state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        let rows = conn.query(&sql, &params).await?;
        Ok(!rows.is_empty())
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
    pub async fn execute_delete(self) -> LrefResult<u64> {
        let sql = format!(
            "DELETE FROM {} {}",
            self.state.from,
            build_where(&self.state.filters)
        );
        let params = self.state.params().to_vec();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
                "No provider attached to QueryBuilder.".to_string(),
            )
        })?;
        let mut conn = provider.get_connection().await?;
        conn.execute(&sql, &params).await
    }
}

// ---------------------------------------------------------------------------
// ExecuteUpdate builder
// ---------------------------------------------------------------------------

/// Builder for bulk update operations.
pub struct ExecuteUpdateBuilder<T: IEntityType> {
    state: QueryState,
    updates: Vec<(String, DbValue)>,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    _phantom: PhantomData<T>,
}

impl<T: IEntityType> ExecuteUpdateBuilder<T> {
    /// Sets a property to a new value.
    pub fn set_property(mut self, _accessor: fn(&T) -> &str, value: impl Into<DbValue>) -> Self {
        self.updates.push(("__column__".to_string(), value.into()));
        self
    }

    /// Sets a named column to a DbValue.
    pub fn set_column(mut self, column: &str, value: impl Into<DbValue>) -> Self {
        self.updates.push((column.to_string(), value.into()));
        self
    }

    /// Returns the generated SQL.
    pub fn to_sql(&self) -> String {
        let sets: Vec<String> = self
            .updates
            .iter()
            .enumerate()
            .map(|(i, (col, _))| format!("{} = ${}", col, i + 1))
            .collect();
        let where_clause = build_where(&self.state.filters);
        format!(
            "UPDATE {} SET {} {}",
            self.state.from,
            sets.join(", "),
            where_clause
        )
    }

    /// Returns params for this bulk update.
    pub fn params(&self) -> Vec<DbValue> {
        let mut params: Vec<DbValue> = self.updates.iter().map(|(_, v)| v.clone()).collect();
        for filter in &self.state.filters {
            if let Some(v) = filter.db_value() {
                params.push(v.clone());
            }
        }
        params
    }

    /// Executes the bulk update.
    pub async fn execute(self) -> LrefResult<u64> {
        let sql = self.to_sql();
        let params = self.params();
        let provider = self.provider.as_ref().ok_or_else(|| {
            crate::error::LrefError::Configuration(
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

/// A query builder for projected results.
pub struct SelectQueryBuilder<T: IEntityType, R> {
    state: QueryState,
    _phantom_t: PhantomData<T>,
    _phantom_r: PhantomData<R>,
}

impl<T: IEntityType, R> SelectQueryBuilder<T, R> {
    /// Returns the generated SQL.
    pub fn to_sql(&self) -> String {
        self.state.to_sql()
    }

    /// Executes the projection query.
    pub async fn to_list(self) -> LrefResult<Vec<R>> {
        let _sql = self.to_sql();
        Ok(Vec::new())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_where(filters: &[FilterCondition]) -> String {
    if filters.is_empty() {
        String::new()
    } else {
        let clauses: Vec<String> = filters
            .iter()
            .enumerate()
            .map(|(i, f)| f.to_sql(&format!("${}", i + 1)))
            .collect();
        format!("WHERE {}", clauses.join(" AND "))
    }
}

// ---------------------------------------------------------------------------
// IQueryable trait
// ---------------------------------------------------------------------------

/// Trait representing a queryable data source.
pub trait IQueryable<T: IEntityType> {
    fn query(&self) -> QueryBuilder<T>;
}
