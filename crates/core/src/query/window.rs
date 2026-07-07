//! Window function specifications (v1.1).
//!
//! `WindowFuncKind` enumerates the supported ranking/aggregate/offset
//! functions; `WindowSpec` carries one window function projection and
//! compiles to a SELECT-list fragment at SQL generation time using the
//! provider's identifier quoting.

use super::ast::OrderDirection;

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
/// Mirrors the design of [`super::ast::HavingExpr`]: a structured AST node
/// stored in [`super::state::QueryState`] and compiled to SQL at generation
/// time so that dialect-specific identifier quoting is applied consistently.
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
