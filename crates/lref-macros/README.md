# lref-macros

[![Crates.io](https://img.shields.io/crates/v/lref-macros)](https://crates.io/crates/lref-macros)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

Procedural macros for [Rust Entity Framework (lref)](https://crates.io/crates/lref).

Provides:

- `#[derive(EntityType)]` — generates `EntityType`, `FromRow`, `GetKeyValues`, and `EntitySnapshot` trait implementations
- `column!()` — resolves entity fields to database column names at compile time

---

## Usage

```rust
use lref::prelude::*;
use lref::column;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,

    #[required]
    #[max_length(200)]
    pub url: String,

    pub rating: i32,
}

// Type-safe column reference
let col = column!(Blog::url);  // expands to Blog::COLUMN_URL → "url"
```

---

## Generated Code

For each struct, `#[derive(EntityType)]` generates:

1. **`EntityType`** impl — returns `EntityTypeMeta` with table name, columns, keys, navigations
2. **`FromRow`** impl — materializes entities from `&[String]` database row data
3. **`GetKeyValues`** impl — returns primary key values for SaveChanges WHERE clauses
4. **`EntitySnapshot`** impl — returns all scalar property values for INSERT/UPDATE
5. **`COLUMN_*`** constants — type-safe column name references (used by `column!()`)

## Supported Field Attributes

| Attribute | Description | Example |
|-----------|-------------|---------|
| `#[primary_key]` | Primary key column | `pub id: i32` |
| `#[auto_increment]` | Auto-increment / identity | `pub id: i32` |
| `#[required]` | NOT NULL constraint | `pub name: String` |
| `#[max_length(200)]` | Max string length | `pub url: String` |
| `#[column("db_col")]` | Different DB column name | `pub rust_name: String` |
| `#[foreign_key(User)]` | Foreign key reference | `pub user_id: i32` |
| `#[navigation]` | Navigation property | `pub blog: BelongsTo<Blog>` |
| `#[not_mapped]` | Exclude from mapping | `pub temp: String` |
| `#[index]` | Create database index | `pub email: String` |
| `#[unique]` | Create unique index | `pub username: String` |
| `#[concurrency_check]` | Optimistic concurrency token | `pub version: i32` |

## License

MIT — see [LICENSE](../../LICENSE)
