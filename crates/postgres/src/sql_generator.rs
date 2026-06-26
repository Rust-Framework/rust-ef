//! PostgreSQL-specific SQL dialect generator.

use rust_ef::provider::ISqlGenerator;

/// SQL generator that produces PostgreSQL-compatible SQL.
#[derive(Debug, Clone)]
pub struct PostgresSqlGenerator;

impl PostgresSqlGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PostgresSqlGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ISqlGenerator for PostgresSqlGenerator {
    fn select(&self, table: &str, columns: &[&str]) -> String {
        let cols = if columns.is_empty() {
            "*".to_string()
        } else {
            columns
                .iter()
                .map(|c| self.quote_identifier(c))
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!("SELECT {} FROM {}", cols, self.quote_identifier(table))
    }

    fn insert(&self, table: &str, columns: &[&str], returning: bool) -> String {
        let cols = columns
            .iter()
            .map(|c| self.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders: Vec<String> = (1..=columns.len())
            .map(|i| self.parameter_placeholder(i))
            .collect();
        let returning_clause = if returning {
            " RETURNING *".to_string()
        } else {
            String::new()
        };
        format!(
            "INSERT INTO {} ({}) VALUES ({}){}",
            self.quote_identifier(table),
            cols,
            placeholders.join(", "),
            returning_clause
        )
    }

    fn update(&self, table: &str, set_columns: &[&str], where_clause: &str) -> String {
        let sets: Vec<String> = set_columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                format!(
                    "{} = {}",
                    self.quote_identifier(c),
                    self.parameter_placeholder(i + 1)
                )
            })
            .collect();
        format!(
            "UPDATE {} SET {} WHERE {}",
            self.quote_identifier(table),
            sets.join(", "),
            where_clause
        )
    }

    fn delete(&self, table: &str, where_clause: &str) -> String {
        format!(
            "DELETE FROM {} WHERE {}",
            self.quote_identifier(table),
            where_clause
        )
    }

    fn create_table(&self, table: &str, columns: &[(String, String)]) -> String {
        let col_defs: Vec<String> = columns
            .iter()
            .map(|(name, type_def)| format!("{} {}", self.quote_identifier(name), type_def))
            .collect();
        format!(
            "CREATE TABLE {} (\n    {}\n)",
            self.quote_identifier(table),
            col_defs.join(",\n    ")
        )
    }

    fn drop_table(&self, table: &str) -> String {
        format!("DROP TABLE IF EXISTS {}", self.quote_identifier(table))
    }

    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String {
        match (skip, take) {
            (Some(s), Some(t)) => format!("OFFSET {} LIMIT {}", s, t),
            (Some(s), None) => format!("OFFSET {}", s),
            (None, Some(t)) => format!("LIMIT {}", t),
            (None, None) => String::new(),
        }
    }

    fn parameter_placeholder(&self, index: usize) -> String {
        format!("${}", index)
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }

    fn auto_increment_syntax(&self) -> &'static str {
        "SERIAL"
    }
}
