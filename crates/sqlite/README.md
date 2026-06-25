# lref-provider-sqlite

[![Crates.io](https://img.shields.io/crates/v/lref-provider-sqlite)](https://crates.io/crates/lref-provider-sqlite)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

SQLite database provider for [Rust Entity Framework (lref)](https://crates.io/crates/lref).

Implements `DatabaseProvider`, `SqlGenerator`, and `AsyncConnection` traits for SQLite using `rusqlite` with a `tokio::sync::Mutex` async wrapper.

---

## Features

- Async-safe via `Arc<Mutex<Connection>>`
- File-based and in-memory (`:memory:`) databases
- WAL mode enabled by default for better concurrent read performance
- SQLite-native parameterized queries (`?`)
- Double-quoted identifier quoting (`"table_name"`)
- Full CRUD, transactions, `AUTOINCREMENT`
- Bundled SQLite (`features = ["bundled"]`) — no system SQLite required

---

## Quick Start

```toml
[dependencies]
lref = "0.1"
lref-provider-sqlite = "0.1"
tokio = { version = "1", features = ["full"] }
```

```rust
use lref::provider::DatabaseProvider;
use lref_provider_sqlite::SqliteProvider;
use std::sync::Arc;

// In-memory database
let provider = Arc::new(SqliteProvider::new_in_memory()?);

// Or file-based
// let provider = Arc::new(SqliteProvider::new("myapp.db")?);

// Get a connection
let mut conn = provider.get_connection().await?;

// Execute parameterized query
conn.execute(
    "INSERT INTO blogs (url, rating) VALUES (?, ?)",
    &[DbValue::String("https://example.com".into()), DbValue::I32(5)],
).await?;

// Query rows
let rows = conn.query(
    "SELECT * FROM blogs WHERE rating > ?",
    &[DbValue::I32(3)],
).await?;
```

---

## Full Example (In-Memory CRUD)

```rust
use lref::prelude::*;
use lref_provider_sqlite::SqliteProvider;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), LrefError> {
    let provider = Arc::new(SqliteProvider::new_in_memory()?);

    // Create table
    let engine = lref::migration::MigrationEngine::new(
        lref::migration::MigrationDialect::Sqlite
    );
    let migration = engine.generate("init", &[Blog::entity_meta()], &None)?;
    provider.execute_migration_command(&migration.up_sql).await?;

    // CRUD operations via DbSet
    let mut db_set = DbSet::<Blog>::with_provider("blogs", provider.clone());

    // INSERT
    db_set.add(Blog { blog_id: 0, url: "https://example.com".into(), rating: 5, posts: HasMany::new() });
    lref::db_context::save_one_set(...).await?;

    // SELECT
    let blogs = db_set.query()
        .filter_column("rating", ">", 3)
        .to_list().await?;

    Ok(())
}
```

---

## SQL Dialect

- **Placeholders**: `?` (anonymous)
- **Identifiers**: `"table_name"` (double-quote)
- **Pagination**: `LIMIT t OFFSET s`
- **Auto-increment**: `AUTOINCREMENT`

---

## License

MIT — see [LICENSE](../../LICENSE)
