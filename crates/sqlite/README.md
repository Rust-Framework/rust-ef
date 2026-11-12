# rust-ef-sqlite

[![crates.io](https://img.shields.io/crates/v/rust-ef-sqlite.svg)](https://crates.io/crates/rust-ef-sqlite)
[![docs.rs](https://docs.rs/rust-ef-sqlite/badge.svg)](https://docs.rs/rust-ef-sqlite)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE)

SQLite provider for [`rust-ef`](https://crates.io/crates/rust-ef), an
EF Core-inspired ORM for Rust.

- Bundled SQLite (`rusqlite 0.41` / `libsqlite3-sys 0.39`, SQLite 3.54+)
- `r2d2` connection pool with the in-tree SQLite manager (no `r2d2_sqlite`)
- WAL mode and busy-timeout handling
- File-based and in-memory databases

## Install

```toml
[dependencies]
rust-ef = "2.2"
rust-ef-sqlite = "2.2"
rust-dix = "0.10"
tokio = { version = "1", features = ["full"] }
```

MSRV: **Rust 1.95**.

## Register with rust-dix

```rust
use rust_dix::*;
use rust_ef::di::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

let mut ctx: rust_ef::db_context::DbContext = provider.get_owned().unwrap();
```

## Options

```rust
// File database with a single pooled connection (serialized writes)
options.use_sqlite_with("app.db", |pool| {
    pool.max_pool_size(1);
});

// In-memory database (fresh database per connection/scope)
options.use_sqlite_in_memory();
```

> `:memory:` cannot be combined with a pooled provider: use `use_sqlite_in_memory()`
> or `SqliteProvider::new_in_memory()`. `new_with_pool(":memory:", n)` is rejected.

## Direct provider use (without DI)

```rust
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::{DbContextOptionsBuilderExt as _, SqliteProvider};

// Standalone provider
let provider = SqliteProvider::new("app.db")?;
// or: SqliteProvider::new_with_pool("app.db", 4)?
// or: SqliteProvider::new_in_memory()?

let mut builder = DbContextOptionsBuilder::new();
builder.use_sqlite("app.db");
let mut ctx = DbContext::from_options(&builder.build())?;
ctx.ensure_created().await?;
```

## SQL dialect

| Aspect | SQLite |
|--------|--------|
| Placeholders | `?` |
| Identifier quoting | `"table"` |
| Pagination | `LIMIT n OFFSET m` |
| Auto-increment | `INTEGER PRIMARY KEY AUTOINCREMENT` |

## License

MIT — see [LICENSE](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE).
