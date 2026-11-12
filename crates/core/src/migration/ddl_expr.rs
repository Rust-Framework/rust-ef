//! Rendering of self-contained boolean expressions into DDL literals.
//!
//! Index predicates (`WHERE ...` for partial indexes) cannot use bound
//! parameters, so the inline values carried by `linq!(|b: T| ...)` are
//! rendered as SQL literals for the target dialect.

use super::dialect::MigrationDialect;
use crate::provider::DbValue;
use crate::query::{BoolExpr, FilterCondition, InSubquerySpec, SubquerySpec};

/// Renders a `BoolExpr` as a DDL boolean expression.
///
/// The expression is intended for partial-index predicates and therefore only
/// supports simple comparisons joined by `AND`/`OR`/`NOT`. Subqueries are
/// rendered on a best-effort basis (they are rarely useful in a partial-index
/// predicate and PostgreSQL forbids them there).
pub(super) fn render_filter(expr: &BoolExpr, dialect: MigrationDialect) -> String {
    match expr {
        BoolExpr::Filter(f) => render_filter_condition(f, dialect),
        BoolExpr::Raw(raw) => raw.0.clone(),
        BoolExpr::And(a, b) => format!(
            "({}) AND ({})",
            render_filter(a, dialect),
            render_filter(b, dialect)
        ),
        BoolExpr::Or(a, b) => format!(
            "({}) OR ({})",
            render_filter(a, dialect),
            render_filter(b, dialect)
        ),
        BoolExpr::Not(inner) => format!("NOT ({})", render_filter(inner, dialect)),
        BoolExpr::Exists(spec) => render_exists(spec, dialect, false),
        BoolExpr::NotExists(spec) => render_exists(spec, dialect, true),
        BoolExpr::InSubquery(spec) => render_in_subquery(spec, dialect, false),
        BoolExpr::NotInSubquery(spec) => render_in_subquery(spec, dialect, true),
    }
}

fn render_filter_condition(f: &FilterCondition, dialect: MigrationDialect) -> String {
    let mut placeholders: Vec<String> =
        f.values().iter().map(|v| sql_literal(v, dialect)).collect();
    // Self-contained `linq!(...)` conditions carry their values. A
    // hand-built `BoolExpr::filter(...)` may not — pad with NULL rather than
    // panicking inside `to_sql`.
    while placeholders.len() < f.param_count() {
        placeholders.push("NULL".to_string());
    }
    f.to_sql(&placeholders)
}

fn render_exists(spec: &SubquerySpec, dialect: MigrationDialect, negated: bool) -> String {
    // Unresolved specs (empty table names) cannot be rendered faithfully.
    if spec.related_table.is_empty() || spec.outer_table.is_empty() {
        return "1=1".into();
    }
    let q = |s: &str| dialect.quote(s);
    let header = if negated { "NOT EXISTS" } else { "EXISTS" };
    let mut sql = format!(
        "{header} (SELECT 1 FROM {} WHERE {}.{} = {}.{}",
        q(&spec.related_table),
        q(&spec.related_table),
        q(&spec.fk_column),
        q(&spec.outer_table),
        q(&spec.outer_pk_column)
    );
    if let Some(pred) = &spec.predicate {
        sql.push_str(&format!(" AND {}", render_filter(pred, dialect)));
    }
    sql.push(')');
    sql
}

fn render_in_subquery(spec: &InSubquerySpec, dialect: MigrationDialect, negated: bool) -> String {
    let q = |s: &str| dialect.quote(s);
    let kw = if negated { "NOT IN" } else { "IN" };
    let mut sql = format!(
        "{} {kw} (SELECT {} FROM {}",
        q(&spec.outer_column),
        q(&spec.projection_column),
        q(&spec.source_table)
    );
    if let Some(pred) = &spec.predicate {
        sql.push_str(&format!(" WHERE {}", render_filter(pred, dialect)));
    }
    sql.push(')');
    sql
}

/// Renders a parameter value as a SQL literal for `dialect`.
pub(super) fn sql_literal(value: &DbValue, dialect: MigrationDialect) -> String {
    match value {
        DbValue::Null => "NULL".to_string(),
        DbValue::Bool(b) => match dialect {
            MigrationDialect::Postgres => {
                if *b {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
            _ => {
                if *b {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
        },
        DbValue::I16(v) => v.to_string(),
        DbValue::I32(v) => v.to_string(),
        DbValue::I64(v) => v.to_string(),
        DbValue::F32(v) => v.to_string(),
        DbValue::F64(v) => v.to_string(),
        DbValue::String(v) => quote_string(v),
        DbValue::Bytes(v) => {
            let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
            match dialect {
                MigrationDialect::MySql => format!("X'{hex}'"),
                MigrationDialect::Postgres | MigrationDialect::Sqlite => format!("X'{hex}'"),
            }
        }
        #[cfg(feature = "chrono")]
        DbValue::DateTime(v) => quote_string(&v.to_rfc3339()),
        #[cfg(feature = "chrono")]
        DbValue::NaiveDateTime(v) => quote_string(&v.to_string()),
        #[cfg(feature = "chrono")]
        DbValue::NaiveDate(v) => quote_string(&v.to_string()),
        #[cfg(feature = "uuid")]
        DbValue::Uuid(v) => quote_string(&v.to_string()),
        #[cfg(feature = "decimal")]
        DbValue::Decimal(v) => quote_string(&v.to_string()),
    }
}

fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
