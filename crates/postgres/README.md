# rust-ef-postgres

[![crates.io](https://img.shields.io/crates/v/rust-ef-postgres.svg)](https://crates.io/crates/rust-ef-postgres)
[![docs.rs](https://docs.rs/rust-ef-postgres/badge.svg)](https://docs.rs/rust-ef-postgres)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE)

PostgreSQL provider for [`rust-ef`](https://crates.io/crates/rust-ef), an
EF Core-inspired ORM for Rust.

- Connection pooling via `deadpool-postgres`
- Positional placeholders (`$1`, `$2`, …)
- `RETURNING *` for auto-increment backfill (`SERIAL` / `BIGSERIAL`)
- Native TLS support (`PgTlsMode`)
- Schema introspection (`information_schema`)

## Install

```toml
[dependencies]
rust-ef = "2.2"
rust-ef-postgres = "2.2"
rust-dix = "0.10"
tokio = { version = "1", features = ["full"] }
```

MSRV: **Rust 1.95**.

## Register with rust-dix

```rust
use rust_dix::*;
use rust_ef::di::*;
use rust_ef_postgres::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_postgres("postgres://user:pass@localhost/app");
    })
    .build()
    .unwrap();

let mut ctx: rust_ef::db_context::DbContext = provider.get_owned().unwrap();
```

## Options

```rust
// Pool size (0 = deadpool default)
options.use_postgres_with_pool("postgres://user:pass@localhost/app", 20);

// Delegate-based configuration
options.use_postgres_with("postgres://user:pass@localhost/app", |pool| {
    pool.pool_size(20);
});

// Explicit TLS mode
options.use_postgres_with_tls("postgres://user:pass@localhost/app", 5, tls);
```

## Direct provider use (without DI)

```rust
use rust_ef_postgres::{PostgresProvider, introspection::introspect_postgres};

let provider = PostgresProvider::new("postgres://user:pass@localhost/app", 5)?;
let tables = introspect_postgres("postgres://user:pass@localhost/app").await?;
```

## SQL dialect

| Aspect | PostgreSQL |
|--------|-----------|
| Placeholders | `$1`, `$2`, … |
| Identifier quoting | `"table"` |
| Pagination | `LIMIT n OFFSET m` |
| Auto-increment | `SERIAL` / `BIGSERIAL` |

## License

MIT — see [LICENSE](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE).
