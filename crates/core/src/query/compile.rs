//! SQL compilation helpers and the portable placeholder generator.
//!
//! `compile_bool_expr` compiles a `BoolExpr` tree into a parameterized SQL
//! WHERE fragment using the provider's `ISqlGenerator` for placeholder
//! syntax. `collect_bool_expr_values` extracts inline parameter values from
//! self-contained `FilterCondition`s produced by `linq!(filter |b: T| ...)`.
//!
//! `PortablePlaceholderGenerator` is a fallback `ISqlGenerator` used for
//! SQL-only queries without an attached provider (emits `?` placeholders and
//! `"identifier"` quoting).

use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, DbValueConvertError};

use super::ast::{BoolExpr, FilterCondition, InSubquerySpec, SubquerySpec};

// ---------------------------------------------------------------------------
// Portable placeholder generator
// ---------------------------------------------------------------------------

/// Fallback generator for SQL-only queries without an attached provider.
pub(crate) struct PortablePlaceholderGenerator;

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
// Aggregate cell conversion
// ---------------------------------------------------------------------------

/// Converts the first cell of the first row of an aggregation query result
/// into the target type `V`. Returns `None` when no rows, no cells, or a SQL
/// NULL was returned (e.g. `MIN`/`MAX` over an empty input set). The driver
/// returns `String` cells, so the value is wrapped in `DbValue::String`
/// before `TryFrom` conversion.
pub(crate) fn convert_aggregate_cell<V>(rows: Vec<Vec<String>>) -> EFResult<Option<V>>
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

// ---------------------------------------------------------------------------
// BoolExpr compilation
// ---------------------------------------------------------------------------

/// Compiles a `BoolExpr` tree into a SQL WHERE fragment using the provider's
/// placeholder syntax. `param_idx` is advanced past each bound parameter in
/// left-to-right order so callers can continue emitting contiguous
/// placeholders (e.g. for HAVING after WHERE).
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

// ---------------------------------------------------------------------------
// Subquery resolution
// ---------------------------------------------------------------------------

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
pub(crate) fn has_subqueries(expr: &BoolExpr) -> bool {
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

// ---------------------------------------------------------------------------
// Legacy `filters` vector compilation
// ---------------------------------------------------------------------------

/// Combines a slice of `FilterCondition`s into a single `BoolExpr` tree
/// joined by `AND`. Used by `or_where` to fold legacy `state.filters` into
/// the boolean expression tree.
pub(crate) fn filters_to_and_expr(filters: &[FilterCondition]) -> BoolExpr {
    filters
        .iter()
        .cloned()
        .map(BoolExpr::Filter)
        .reduce(|acc, f| BoolExpr::And(Box::new(acc), Box::new(f)))
        .unwrap_or(BoolExpr::Raw("1=1".to_string()))
}

/// Builds a WHERE clause from `FilterCondition`s starting at `param_idx = 1`.
pub(crate) fn build_where_clauses(
    filters: &[FilterCondition],
    gen: &dyn crate::provider::ISqlGenerator,
) -> String {
    build_where_clause_with_offset(filters, gen, 1)
}

/// Builds a WHERE clause from `FilterCondition`s starting at `start_index`.
/// Used by `ExecuteUpdateBuilder::to_sql` to continue placeholder numbering
/// after the SET clause's parameters.
pub(crate) fn build_where_clause_with_offset(
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
