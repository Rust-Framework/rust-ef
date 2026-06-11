//! Rust type to PostgreSQL type mapping.

pub struct PostgresTypeMapping;

impl PostgresTypeMapping {
    pub fn map_type(rust_type: &str) -> &'static str {
        match rust_type {
            "i16" => "SMALLINT",
            "i32" => "INTEGER",
            "i64" => "BIGINT",
            "u16" => "SMALLINT",
            "u32" => "INTEGER",
            "u64" => "BIGINT",
            "f32" => "REAL",
            "f64" => "DOUBLE PRECISION",
            "bool" => "BOOLEAN",
            "String" => "TEXT",
            "Vec<u8>" => "BYTEA",
            _ => "TEXT",
        }
    }

    pub fn map_auto_increment_type(rust_type: &str) -> &'static str {
        match rust_type {
            "i16" => "SMALLSERIAL",
            "i32" => "SERIAL",
            "i64" => "BIGSERIAL",
            _ => "SERIAL",
        }
    }

    pub fn column_definition(
        _column_name: &str,
        rust_type: &str,
        is_required: bool,
        is_auto_increment: bool,
        max_length: Option<usize>,
    ) -> String {
        let pg_type = if is_auto_increment {
            Self::map_auto_increment_type(rust_type).to_string()
        } else {
            let base_type = Self::map_type(rust_type);
            match (base_type, max_length) {
                ("TEXT", Some(n)) => format!("VARCHAR({})", n),
                (t, _) => t.to_string(),
            }
        };

        if is_required {
            format!("{} NOT NULL", pg_type)
        } else {
            pg_type
        }
    }
}
