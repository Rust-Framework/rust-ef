# lref-provider-postgres

[![Crates.io](https://img.shields.io/crates/v/lref-provider-postgres)](https://crates.io/crates/lref-provider-postgres)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

PostgreSQL database provider for [Rust Entity Framework (lref)](https://crates.io/crates/lref).

Implements `DatabaseProvider`, `SqlGenerator`, and `AsyncConnection` traits for PostgreSQL using `deadpool-postgres` for connection pooling.

---

## Features

- Connection pooling via `deadpool-postgres`
- PostgreSQL-native parameterized queries (`$1`, `$2`, ...)
- Double-quoted identifier quoting (`"table_name"`)
- `RETURNING *` support for INSERT auto-increment backfill
- `SERIAL` / `BIGSERIAL` type mapping for auto-increment columns
- Full CRUD: `execute()`, `query()`, transactions
- Database introspection (`information_schema.columns` + `table_constraints`)

---

## Quick Start

```toml
[dependencies]
lref = "0.1"
lref-provider-postgres = "0.1"
tokio = { version = "1", features = ["full"] }
```

```rust
use lref::provider::DatabaseProvider;
use lref_provider_postgres::PostgresProvider;
use std::sync::Arc;

let provider = Arc::new(PostgresProvider::new(
    "postgres://user:password@localhost/mydb",
    5,  // pool size
)?);

// Get a connection
let mut conn = provider.get_connection().await?;

// Execute parameterized query
conn.execute("INSERT INTO blogs (url, rating) VALUES ($1, $2)", &[
    DbValue::String("https://example.com".into()),
    DbValue::I32(5),
]).await?;

// Query rows
let rows = conn.query("SELECT * FROM blogs WHERE rating > $1", &[
    DbValue::I32(3),
]).await?;
```

---

## Type Mapping

| Rust Type | PostgreSQL Type |
|-----------|----------------|
| `i16` | `SMALLINT` / `SMALLSERIAL` |
| `i32` | `INTEGER` / `SERIAL` |
| `i64` | `BIGINT` / `BIGSERIAL` |
| `f32` | `REAL` |
| `f64` | `DOUBLE PRECISION` |
| `bool` | `BOOLEAN` |
| `String` | `TEXT` / `VARCHAR(n)` |
| `Vec<u8>` | `BYTEA` |

---

## Database Introspection

```rust
use lref_provider_postgres::introspection::introspect_postgres;

let tables = introspect_postgres(
    "postgres://user:pass@localhost/mydb"
).await?;

for table in &tables {
    println!("Table: {}", table.name);
    for col in &table.columns {
        println!("  {}: {} (nullable={}, pk={})",
            col.name, col.data_type, col.is_nullable, col.is_primary_key);
    }
}
```

---

## SQL Dialect

- **Placeholders**: `$1`, `$2`, ... (positional)
- **Identifiers**: `"table_name"` (double-quote)
- **Pagination**: `OFFSET N LIMIT M`
- **Auto-increment**: `SERIAL`

---

## License

MIT — see [LICENSE](../../LICENSE)
