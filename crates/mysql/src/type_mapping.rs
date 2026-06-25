pub struct MySqlTypeMapping;

impl MySqlTypeMapping {
    pub fn map_type(rust_type: &str) -> &'static str {
        match rust_type {
            "i16" => "SMALLINT",
            "i32" => "INT",
            "i64" => "BIGINT",
            "u16" => "SMALLINT UNSIGNED",
            "u32" => "INT UNSIGNED",
            "u64" => "BIGINT UNSIGNED",
            "f32" => "FLOAT",
            "f64" => "DOUBLE",
            "bool" => "BOOLEAN",
            "String" => "TEXT",
            "Vec<u8>" => "BLOB",
            _ => "TEXT",
        }
    }

    pub fn map_auto_increment_type(rust_type: &str) -> &'static str {
        match rust_type {
            "i16" => "SMALLINT AUTO_INCREMENT",
            "i32" => "INT AUTO_INCREMENT",
            "i64" => "BIGINT AUTO_INCREMENT",
            _ => "INT AUTO_INCREMENT",
        }
    }

    pub fn column_definition(
        _column_name: &str,
        rust_type: &str,
        is_required: bool,
        is_auto_increment: bool,
        max_length: Option<usize>,
    ) -> String {
        let base_type = if is_auto_increment {
            Self::map_auto_increment_type(rust_type).to_string()
        } else {
            let base = Self::map_type(rust_type);
            match (base, max_length) {
                ("TEXT", Some(n)) => format!("VARCHAR({})", n),
                (t, _) => t.to_string(),
            }
        };

        if is_required {
            format!("{} NOT NULL", base_type)
        } else {
            base_type
        }
    }
}
