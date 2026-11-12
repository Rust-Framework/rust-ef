# rust-ef-mysql

[![crates.io](https://img.shields.io/crates/v/rust-ef-mysql.svg)](https://crates.io/crates/rust-ef-mysql)
[![docs.rs](https://docs.rs/rust-ef-mysql/badge.svg)](https://docs.rs/rust-ef-mysql)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE)

MySQL provider for [`rust-ef`](https://crates.io/crates/rust-ef), an
EF Core-inspired ORM for Rust.

- `sqlx-mysql 0.9` connection pooling (no `sqlx` umbrella; see 2.0.1 security notes)
- MySQL-native placeholders (`?`) and backtick identifier quoting
- `AUTO_INCREMENT` column type
- `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4` table creation
- RSA (`caching_sha2_password`) and TLS authentication support
- Schema introspection

## Install

```toml
[dependencies]
rust-ef = "2.2"
rust-ef-mysql = "2.2"
rust-dix = "0.10"
tokio = { version = "1", features = ["full"] }
```

MSRV: **Rust 1.95**.

## Register with rust-dix

```rust
use rust_dix::*;
use rust_ef::di::*;
use rust_ef_mysql::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_mysql("mysql://user:pass@localhost/app");
    })
    .build()
    .unwrap();

let mut ctx: rust_ef::db_context::DbContext = provider.get_owned().unwrap();
```

## Options

```rust
// Delegate-based configuration
options.use_mysql_with("mysql://user:pass@localhost/app", |o| {
    o.max_pool_size(20);
});

// Pool callback (sqlx-mysql 0.9 `MySqlPoolOptions`)
options.use_mysql_with_pool(
    "mysql://user:pass@localhost/app",
    std::sync::Arc::new(|o| { o.max_connections(20); }),
);

// Explicit TLS mode
options.use_mysql_with_tls("mysql://user:pass@localhost/app", MySqlTlsMode::Required);
```

> Requires `sqlx-mysql 0.9` if you build pools yourself. If you pass a pool built
> with `sqlx 0.8` to `MySqlProvider::from_pool` / `use_mysql_with_pool`, align your
> `sqlx` dependency to 0.9.

## Direct provider use (without DI)

```rust
use rust_ef_mysql::{MySqlProvider, introspection::introspect_mysql};

let provider = MySqlProvider::new("mysql://user:pass@localhost/app").await?;
let tables = introspect_mysql("mysql://user:pass@localhost/app").await?;
```

## SQL dialect

| Aspect | MySQL |
|--------|-------|
| Placeholders | `?` |
| Identifier quoting | `` `table` `` |
| Pagination | `LIMIT n OFFSET m` |
| Auto-increment | `AUTO_INCREMENT` |
| Tables | `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4` |

## License

MIT — see [LICENSE](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE).
