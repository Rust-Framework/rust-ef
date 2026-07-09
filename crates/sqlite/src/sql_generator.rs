use rust_ef::provider::ISqlGenerator;

#[derive(Debug, Clone)]
pub struct SqliteSqlGenerator;

impl SqliteSqlGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SqliteSqlGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ISqlGenerator for SqliteSqlGenerator {
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

    fn insert(&self, table: &str, columns: &[&str], _returning: bool) -> String {
        let cols = columns
            .iter()
            .map(|c| self.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = vec!["?"; columns.len()].join(", ");
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            self.quote_identifier(table),
            cols,
            placeholders
        )
    }

    fn insert_batch(&self, table: &str, columns: &[&str], row_count: usize) -> String {
        let cols = columns
            .iter()
            .map(|c| self.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        let row = format!("({})", vec!["?"; columns.len()].join(", "));
        let all_rows = vec![row; row_count].join(", ");
        format!(
            "INSERT INTO {} ({}) VALUES {}",
            self.quote_identifier(table),
            cols,
            all_rows,
        )
    }

    fn upsert_batch(
        &self,
        table: &str,
        columns: &[&str],
        conflict_cols: &[&str],
        row_count: usize,
    ) -> String {
        let cols = columns
            .iter()
            .map(|c| self.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        let row = format!("({})", vec!["?"; columns.len()].join(", "));
        let all_rows = vec![row; row_count].join(", ");
        let conflict = conflict_cols
            .iter()
            .map(|c| self.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        let update_cols: Vec<&str> = columns
            .iter()
            .filter(|c| !conflict_cols.contains(c))
            .copied()
            .collect();
        let sets = update_cols
            .iter()
            .map(|c| {
                format!(
                    "{} = EXCLUDED.{}",
                    self.quote_identifier(c),
                    self.quote_identifier(c)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "INSERT INTO {} ({}) VALUES {} ON CONFLICT({}) DO UPDATE SET {}",
            self.quote_identifier(table),
            cols,
            all_rows,
            conflict,
            sets,
        )
    }

    fn update(&self, table: &str, set_columns: &[&str], where_clause: &str) -> String {
        let sets: Vec<String> = set_columns
            .iter()
            .map(|c| format!("{} = ?", self.quote_identifier(c)))
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
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            _ => String::new(),
        }
    }

    fn parameter_placeholder(&self, _index: usize) -> String {
        "?".to_string()
    }

    fn quote_identifier(&self, identifier: &str) -> String {
        format!("\"{}\"", identifier)
    }

    fn auto_increment_syntax(&self) -> &'static str {
        "AUTOINCREMENT"
    }

    fn supports_returning(&self) -> bool {
        false
    }

    fn last_insert_id_sql(&self) -> Option<&'static str> {
        Some("SELECT last_insert_rowid()")
    }

    fn last_insert_id_returns_first(&self) -> bool {
        false
    }
}
