//! LINQ string pattern helpers.

/// Builds a `%value%` LIKE pattern (EFCore `Contains`).
pub fn like_contains(value: impl AsRef<str>) -> String {
    format!("%{}%", value.as_ref())
}

/// Builds a `value%` LIKE pattern (EFCore `StartsWith`).
pub fn like_starts_with(value: impl AsRef<str>) -> String {
    format!("{}%", value.as_ref())
}

/// Builds a `%value` LIKE pattern (EFCore `EndsWith`).
pub fn like_ends_with(value: impl AsRef<str>) -> String {
    format!("%{}", value.as_ref())
}
