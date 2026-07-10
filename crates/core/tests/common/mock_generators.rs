//! Mock SQL generators mimicking each provider's dialect behavior for
//! HAVING placeholder and LIMIT/OFFSET pagination dialect tests.

use rust_ef::provider::ISqlGenerator;

/// Mimics PostgreSQL: `$N` placeholders, `OFFSET x LIMIT y` pagination order.
pub struct PgLikeGenerator;

impl ISqlGenerator for PgLikeGenerator {
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

/// Mimics MySQL: `?` placeholders, `LIMIT x OFFSET y` order, special
/// offset-only case using `LIMIT 18446744073709551615 OFFSET y`.
pub struct MySqlLikeGenerator;

impl ISqlGenerator for MySqlLikeGenerator {
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
        match (skip, take) {
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            (Some(s), None) => format!("LIMIT 18446744073709551615 OFFSET {}", s),
            (None, None) => String::new(),
        }
    }
    fn parameter_placeholder(&self, _: usize) -> String {
        "?".to_string()
    }
    fn quote_identifier(&self, identifier: &str) -> String {
        format!("`{}`", identifier)
    }
    fn auto_increment_syntax(&self) -> &'static str {
        "AUTO_INCREMENT"
    }
}

/// Mimics SQLite: `?` placeholders, `LIMIT x OFFSET y` order, no offset-only.
pub struct SqliteLikeGenerator;

impl ISqlGenerator for SqliteLikeGenerator {
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
        match (skip, take) {
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            _ => String::new(),
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
