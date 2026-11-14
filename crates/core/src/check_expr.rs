//! Portable `CHECK` expression builders (EF-DDL-02).
//!
//! `has_check(name, expression)` accepts raw SQL, which is powerful but not
//! portable: for example `(a IS NOT NULL) + (b IS NOT NULL) = 1` is valid on
//! SQLite/MySQL and invalid on PostgreSQL (boolean `+`), and `IN (...)` is
//! rendered as `= ANY (ARRAY[...])` by PostgreSQL. Every application then
//! rewrites the same checks per dialect.
//!
//! [`CheckExpr`] builds the common shapes from one portable formulation:
//! `CASE WHEN ... THEN 1 ELSE 0 END` arithmetic works on all three dialects,
//! and the string literals are escaped centrally.
//!
//! ```ignore
//! use rust_ef::check_expr::CheckExpr;
//!
//! builder.has_check_expr(
//!     "ck_channel_scope",
//!     CheckExpr::exactly_one_of(&["tenant_id", "channel_id"]),
//! );
//! builder.has_check_expr(
//!     "ck_order_status",
//!     CheckExpr::in_list("status", travel_contracts::enums::OrderStatus::ALL),
//! );
//! ```

use std::fmt;

/// A portable SQL boolean expression for a `CHECK` constraint.
///
/// The rendered SQL avoids dialect-specific operators, so the same expression
/// is valid on PostgreSQL, MySQL and SQLite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckExpr {
    sql: String,
}

impl CheckExpr {
    /// Wraps a trusted, already-portable expression verbatim.
    pub fn raw(sql: impl Into<String>) -> Self {
        Self { sql: sql.into() }
    }

    /// `column IN ('a', 'b', ...)` with escaped literals.
    ///
    /// An empty list renders `1 = 0` (no value can satisfy it), which is more
    /// portable than `IN (NULL)`.
    pub fn in_list<I, S>(column: &str, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let values: Vec<String> = values
            .into_iter()
            .map(|value| quote_literal(value.as_ref()))
            .collect();
        if values.is_empty() {
            return Self::raw("1 = 0");
        }
        Self::raw(format!("{} IN ({})", identifier(column), values.join(", ")))
    }

    /// `column NOT IN ('a', 'b', ...)`.
    pub fn not_in_list<I, S>(column: &str, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let values: Vec<String> = values
            .into_iter()
            .map(|value| quote_literal(value.as_ref()))
            .collect();
        if values.is_empty() {
            return Self::raw("1 = 1");
        }
        Self::raw(format!(
            "{} NOT IN ({})",
            identifier(column),
            values.join(", ")
        ))
    }

    /// Exactly one of `columns` is `NOT NULL`.
    ///
    /// Portable form: `(CASE WHEN c1 IS NOT NULL THEN 1 ELSE 0 END + ...) = 1`.
    pub fn exactly_one_of(columns: &[&str]) -> Self {
        Self::count_not_null(columns, 1)
    }

    /// At most one of `columns` is `NOT NULL` (zero is allowed).
    pub fn at_most_one_of(columns: &[&str]) -> Self {
        if columns.is_empty() {
            return Self::raw("1 = 1");
        }
        Self::raw(format!("{} <= 1", not_null_sum(columns)))
    }

    /// At least one of `columns` is `NOT NULL`.
    pub fn at_least_one_of(columns: &[&str]) -> Self {
        if columns.is_empty() {
            return Self::raw("1 = 0");
        }
        Self::raw(format!("{} >= 1", not_null_sum(columns)))
    }

    /// All of `columns` are `NOT NULL`.
    pub fn all_not_null(columns: &[&str]) -> Self {
        if columns.is_empty() {
            return Self::raw("1 = 1");
        }
        let parts: Vec<String> = columns
            .iter()
            .map(|column| format!("{} IS NOT NULL", identifier(column)))
            .collect();
        Self::raw(parts.join(" AND "))
    }

    /// `column BETWEEN lower AND upper` for integer bounds.
    pub fn between(column: &str, lower: i64, upper: i64) -> Self {
        Self::raw(format!(
            "{} BETWEEN {} AND {}",
            identifier(column),
            lower,
            upper
        ))
    }

    /// `column >= 0`.
    pub fn non_negative(column: &str) -> Self {
        Self::raw(format!("{} >= 0", identifier(column)))
    }

    /// `column > 0`.
    pub fn positive(column: &str) -> Self {
        Self::raw(format!("{} > 0", identifier(column)))
    }

    /// Combines expressions with `AND`, parenthesizing each operand.
    pub fn all_of(expressions: impl IntoIterator<Item = CheckExpr>) -> Self {
        let parts: Vec<String> = expressions
            .into_iter()
            .map(|expression| format!("({})", expression.sql))
            .collect();
        if parts.is_empty() {
            return Self::raw("1 = 1");
        }
        Self::raw(parts.join(" AND "))
    }

    /// Combines expressions with `OR`, parenthesizing each operand.
    pub fn any_of(expressions: impl IntoIterator<Item = CheckExpr>) -> Self {
        let parts: Vec<String> = expressions
            .into_iter()
            .map(|expression| format!("({})", expression.sql))
            .collect();
        if parts.is_empty() {
            return Self::raw("1 = 0");
        }
        Self::raw(parts.join(" OR "))
    }

    /// The rendered SQL expression (without the enclosing `CHECK (...)`).
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Whether the expression is empty/blank (an invalid `CHECK`).
    pub fn is_blank(&self) -> bool {
        self.sql.trim().is_empty()
    }

    fn count_not_null(columns: &[&str], expected: i64) -> Self {
        if columns.is_empty() {
            return Self::raw("1 = 0");
        }
        Self::raw(format!("{} = {}", not_null_sum(columns), expected))
    }
}

impl fmt::Display for CheckExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.sql)
    }
}

impl From<CheckExpr> for String {
    fn from(expression: CheckExpr) -> Self {
        expression.sql
    }
}

/// `CASE WHEN c1 IS NOT NULL THEN 1 ELSE 0 END + ...`.
fn not_null_sum(columns: &[&str]) -> String {
    let parts: Vec<String> = columns
        .iter()
        .map(|column| {
            format!(
                "(CASE WHEN {} IS NOT NULL THEN 1 ELSE 0 END)",
                identifier(column)
            )
        })
        .collect();
    parts.join(" + ")
}

/// Returns a bare identifier unchanged; wraps one that needs quoting only when
/// it contains no quote character. Model columns are bare identifiers, so this
/// is a guard rather than a full identifier quoter.
fn identifier(name: &str) -> String {
    let name = name.trim();
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// Renders a single-quoted SQL string literal.
fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::CheckExpr;

    #[test]
    fn exactly_one_of_uses_portable_case_arithmetic() {
        let expression = CheckExpr::exactly_one_of(&["tenant_id", "channel_id"]);
        assert_eq!(
            expression.sql(),
            "(CASE WHEN tenant_id IS NOT NULL THEN 1 ELSE 0 END) + \
             (CASE WHEN channel_id IS NOT NULL THEN 1 ELSE 0 END) = 1"
        );
        // No boolean arithmetic: valid on PostgreSQL too.
        assert!(!expression.sql().contains("IS NOT NULL) +"));
    }

    #[test]
    fn in_list_escapes_literals() {
        let expression = CheckExpr::in_list("status", ["A'B", "C"]);
        assert_eq!(expression.sql(), "status IN ('A''B', 'C')");
    }

    #[test]
    fn empty_in_list_is_false_and_empty_not_in_is_true() {
        assert_eq!(CheckExpr::in_list::<_, &str>("status", []).sql(), "1 = 0");
        assert_eq!(
            CheckExpr::not_in_list::<_, &str>("status", []).sql(),
            "1 = 1"
        );
    }

    #[test]
    fn at_most_and_at_least_one() {
        let at_most = CheckExpr::at_most_one_of(&["a", "b"]);
        assert!(at_most.sql().ends_with("<= 1"));
        let at_least = CheckExpr::at_least_one_of(&["a", "b"]);
        assert!(at_least.sql().ends_with(">= 1"));
        assert_eq!(CheckExpr::at_most_one_of(&[]).sql(), "1 = 1");
        assert_eq!(CheckExpr::at_least_one_of(&[]).sql(), "1 = 0");
    }

    #[test]
    fn combinators_parenthesize_operands() {
        let combined = CheckExpr::any_of([
            CheckExpr::non_negative("amount"),
            CheckExpr::positive("version"),
        ]);
        assert_eq!(combined.sql(), "(amount >= 0) OR (version > 0)");
        let all = CheckExpr::all_of([
            CheckExpr::non_negative("amount"),
            CheckExpr::positive("version"),
        ]);
        assert_eq!(all.sql(), "(amount >= 0) AND (version > 0)");
    }

    #[test]
    fn between_and_non_negative() {
        assert_eq!(
            CheckExpr::between("version", 1, 10).sql(),
            "version BETWEEN 1 AND 10"
        );
        assert_eq!(CheckExpr::non_negative("amount").sql(), "amount >= 0");
    }
}
