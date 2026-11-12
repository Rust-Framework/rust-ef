//! Rendering of self-contained boolean expressions into DDL literals.
//!
//! Index predicates (`WHERE ...` for partial indexes) cannot use bound
//! parameters, so the inline values carried by `linq!(|b: T| ...)` are
//! rendered as SQL literals for the target dialect.

use super::dialect::MigrationDialect;
use crate::provider::DbValue;
use crate::query::{BoolExpr, FilterCondition};

/// Renders a `BoolExpr` as a DDL boolean expression.
///
/// The expression is intended for partial-index predicates; values are
/// rendered as inline literals (DDL cannot bind parameters).
///
/// # Panics
///
/// Panics for subquery nodes: no supported dialect allows `EXISTS` / `IN
/// (SELECT ...)` in an index predicate, and rendering them as a constant would
/// silently broaden the index. Also panics when a condition carries fewer
/// values than its parameter count (a hand-built `FilterCondition::new`).
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
        BoolExpr::Exists(_) | BoolExpr::NotExists(_) => {
            panic!("subquery predicates are not supported in index filters")
        }
        BoolExpr::InSubquery(_) | BoolExpr::NotInSubquery(_) => {
            panic!("subquery predicates are not supported in index filters")
        }
    }
}

fn render_filter_condition(f: &FilterCondition, dialect: MigrationDialect) -> String {
    // A self-contained condition must carry one value per bound parameter.
    // `linq!(|b: T| ...)` always does; a hand-built `FilterCondition::new(col,
    // op, n)` does not, and rendering `col = NULL` for it would silently create
    // an index predicate that never matches. Fail fast instead, mirroring
    // `QueryBuilder::apply_query_filter`.
    assert_eq!(
        f.values().len(),
        f.param_count(),
        "self-contained condition is missing bound values: build it with \
         `linq!(|b: T| ...)` or `FilterCondition::with_values`, not \
         `FilterCondition::new`"
    );
    let placeholders: Vec<String> = f.values().iter().map(|v| sql_literal(v, dialect)).collect();
    f.to_sql(&placeholders)
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
        DbValue::F32(v) => render_float_text(v.to_string(), v.is_finite(), "f4"),
        DbValue::F64(v) => render_float_text(v.to_string(), v.is_finite(), "f8"),
        DbValue::String(v) => quote_string(v, dialect),
        // MySQL and SQLite `X'..'` are binary strings; PostgreSQL's is a *bit*
        // string, so bytes must go through `decode(.., 'hex')` there.
        DbValue::Bytes(v) => {
            let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
            match dialect {
                MigrationDialect::Postgres => format!("decode('{hex}', 'hex')"),
                MigrationDialect::MySql | MigrationDialect::Sqlite => format!("X'{hex}'"),
            }
        }
        #[cfg(feature = "chrono")]
        DbValue::DateTime(v) => quote_string(&v.to_rfc3339(), dialect),
        #[cfg(feature = "chrono")]
        DbValue::NaiveDateTime(v) => quote_string(&v.to_string(), dialect),
        #[cfg(feature = "chrono")]
        DbValue::NaiveDate(v) => quote_string(&v.to_string(), dialect),
        #[cfg(feature = "uuid")]
        DbValue::Uuid(v) => quote_string(&v.to_string(), dialect),
        #[cfg(feature = "decimal")]
        DbValue::Decimal(v) => quote_string(&v.to_string(), dialect),
    }
}

/// Floats have no portable spelling for NaN / infinities; refuse to emit an
/// invalid DDL literal instead of producing SQL that fails when applied.
fn render_float_text(text: String, finite: bool, kind: &str) -> String {
    assert!(
        finite,
        "cannot render a non-finite {kind} float as a DDL literal"
    );
    // Keep a decimal point so the literal stays a float in every dialect.
    if text.contains('.') || text.contains('e') || text.contains('E') {
        text
    } else {
        format!("{text}.0")
    }
}

/// Renders a string value as a dialect-correct SQL literal (quote doubling,
/// plus backslash escaping for MySQL).
pub(super) fn quote_string(value: &str, dialect: MigrationDialect) -> String {
    let escaped = match dialect {
        // MySQL treats `\` as an escape character inside string literals
        // (`NO_BACKSLASH_ESCAPES` off by default), so double it; `'` is escaped
        // by doubling in every supported dialect.
        MigrationDialect::MySql => value.replace('\\', "\\\\").replace('\'', "''"),
        MigrationDialect::Postgres | MigrationDialect::Sqlite => value.replace('\'', "''"),
    };
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{BoolExpr, FilterCondition};

    fn condition(column: &str, op: &str, values: Vec<DbValue>) -> BoolExpr {
        BoolExpr::Filter(FilterCondition::with_values(column, op, values))
    }

    #[test]
    fn string_literals_escape_quotes_and_mysql_backslashes() {
        let value = DbValue::String("a'b\\c".into());
        assert_eq!(sql_literal(&value, MigrationDialect::Postgres), "'a''b\\c'");
        assert_eq!(sql_literal(&value, MigrationDialect::Sqlite), "'a''b\\c'");
        assert_eq!(sql_literal(&value, MigrationDialect::MySql), "'a''b\\\\c'");
    }

    #[test]
    fn bytes_render_as_blob_or_postgres_hex_decode() {
        let value = DbValue::Bytes(vec![0xde, 0xad]);
        assert_eq!(sql_literal(&value, MigrationDialect::MySql), "X'dead'");
        assert_eq!(sql_literal(&value, MigrationDialect::Sqlite), "X'dead'");
        assert_eq!(
            sql_literal(&value, MigrationDialect::Postgres),
            "decode('dead', 'hex')"
        );
    }

    #[test]
    fn bool_literals_follow_the_dialect() {
        assert_eq!(
            sql_literal(&DbValue::Bool(true), MigrationDialect::Postgres),
            "TRUE"
        );
        assert_eq!(
            sql_literal(&DbValue::Bool(false), MigrationDialect::MySql),
            "0"
        );
    }

    #[test]
    fn floats_keep_a_decimal_point() {
        assert_eq!(
            sql_literal(&DbValue::F64(2.0), MigrationDialect::Postgres),
            "2.0"
        );
        assert_eq!(
            sql_literal(&DbValue::F64(2.5), MigrationDialect::Postgres),
            "2.5"
        );
    }

    #[test]
    fn compound_filters_keep_parentheses() {
        let expr = condition("a", "=", vec![DbValue::I32(1)]).and(BoolExpr::Or(
            Box::new(condition("b", "=", vec![DbValue::I32(2)])),
            Box::new(condition("c", "=", vec![DbValue::I32(3)])),
        ));
        assert_eq!(
            render_filter(&expr, MigrationDialect::Postgres),
            "(a = 1) AND ((b = 2) OR (c = 3))"
        );
    }

    #[test]
    fn empty_in_list_renders_as_unsatisfiable() {
        let expr = condition("id", "IN", vec![]);
        assert_eq!(render_filter(&expr, MigrationDialect::Sqlite), "1 = 0");
    }

    #[test]
    #[should_panic(expected = "missing bound values")]
    fn valueless_condition_is_rejected() {
        let expr = BoolExpr::Filter(FilterCondition::new("a", "=", 1));
        let _ = render_filter(&expr, MigrationDialect::Postgres);
    }

    #[test]
    #[should_panic(expected = "subquery predicates are not supported")]
    fn subquery_filter_is_rejected_instead_of_broadening_the_index() {
        let spec = rust_ef_query_subquery_spec();
        let expr = BoolExpr::Exists(Box::new(spec));
        let _ = render_filter(&expr, MigrationDialect::Postgres);
    }

    fn rust_ef_query_subquery_spec() -> crate::query::SubquerySpec {
        let mut spec = crate::query::SubquerySpec::new("posts", "Post::TABLE");
        spec.outer_table = "Blog::TABLE".into();
        spec
    }
}
