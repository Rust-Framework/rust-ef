# lref-provider-mysql

[![Crates.io](https://img.shields.io/crates/v/lref-provider-mysql)](https://crates.io/crates/lref-provider-mysql)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

MySQL database provider for [Rust Entity Framework (lref)](https://crates.io/crates/lref).

Implements `DatabaseProvider`, `SqlGenerator`, and `AsyncConnection` traits for MySQL using `sqlx` with async connection pooling.

---

## Features

- Connection pooling via `sqlx::MySqlPool`
- MySQL-native parameterized queries (`?`)
- Backtick identifier quoting (`` `table_name` ``)
- `AUTO_INCREMENT` column type
- Full CRUD: `execute()`, `query()`, transactions (`START TRANSACTION` / `COMMIT` / `ROLLBACK`)
- `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4` table creation

---

## Quick Start

```toml
[dependencies]
lref = "0.1"
lref-provider-mysql = "0.1"
tokio = { version = "1", features = ["full"] }
```

```rust
use lref::provider::DatabaseProvider;
use lref_provider_mysql::MySqlProvider;
use std::sync::Arc;

let provider = Arc::new(MySqlProvider::new(
    "mysql://user:password@localhost/mydb"
).await?);

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

## SQL Dialect

- **Placeholders**: `?` (anonymous)
- **Identifiers**: `` `table_name` `` (backtick)
- **Pagination**: `LIMIT t OFFSET s`
- **Auto-increment**: `AUTO_INCREMENT`
- **Table creation**: `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`

---

## License

MIT  - ?see [LICENSE](../../LICENSE)
