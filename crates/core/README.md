# rust-ef

[![crates.io](https://img.shields.io/crates/v/rust-ef.svg)](https://crates.io/crates/rust-ef)
[![docs.rs](https://docs.rs/rust-ef/badge.svg)](https://docs.rs/rust-ef)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE)

An **EF Core-inspired ORM for Rust** — `DbContext` / `DbSet<T>` / `IEntityType`,
the `linq!` macro, change tracking, transactions, migrations, and multi-database
support, with first-class [`rust-dix` 0.9](https://crates.io/crates/rust-dix)
dependency-injection integration.

Full handbook: <https://rf2026.github.io/rust-ef/>

---

## Install

```toml
[dependencies]
rust-ef = "2.1"
rust-ef-sqlite = "2.1"          # one provider: sqlite / postgres / mysql
rust-dix = "0.10"                # DI container
tokio = { version = "1", features = ["full"] }
```

MSRV: **Rust 1.95**.

## Define an entity

```rust
use rust_ef::prelude::*;

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
    #[navigation]
    pub posts: HasMany<Post>,
}
```

`#[derive(EntityType)]` registers the entity at compile time; `DbContext::from_options()`
discovers it automatically — no manual `discover_entities()` call.

## Register a `DbContext` (rust-dix 0.10)

```rust
use std::sync::Arc;
use rust_dix::*;
use rust_ef::di::*;
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

// Owned resolution (recommended): &mut self access, no locks.
// Allowed from the root provider: the instance is caller-owned and never cached.
let mut ctx: DbContext = provider.get_owned().unwrap();

// Shared resolution: Arc<DbContext>, &self access, one instance per scope.
// rust-dix 0.10 enables strict scope validation by default, so create a scope.
let scope = provider.scope();
let shared: Arc<DbContext> = scope.get().unwrap();
```

> Upgrading from rust-ef 2.0 / rust-dix 0.7? Root `get::<DbContext>()` now returns
> `RdiError::ScopedFromRootProvider`. Use `provider.scope()` (or owned resolution).
> For a temporary transition, build with
> `ServiceCollection::build_with(ServiceProviderOptions::lenient())`.
> See [CHANGELOG.md](https://github.com/Rust-Framework/rust-ef/blob/main/CHANGELOG.md).

### More registration variants (2.1)

- **Typed keys**: `#[derive(DbContextKey)]` generates `IDbContextKey` and the
  rust-dix 0.10 `From<Key> for ServiceKey` contract, so registration,
  `DbContextResolver` and `#[inject(owned, key = Db::Read)]` all use the same
  typed key.
- **Dynamic tenant/sharded contexts**: `add_dbcontext_anykey(|key, options| ...)`
  registers one fallback for any requested key, with bounded-LRU per-key
  provider/pool caching (`add_dbcontext_anykey_with_capacity`).
  Use the default entity model + query filters for isolation.
- **Async startup**: `add_dbcontext_async` / `add_dbcontext_keyed_async` defer
  provider creation to `build_async().await` and return typed `RdiError` failures;
  an optional `options.async_preflight(...)` hook validates the pool asynchronously.
  The `DbContext` stays a synchronous Scoped service, so owned injection is unchanged.

See the handbook DI chapter and `docs/v2.1-migration-guide.md`.

## Query and save

```rust
let mut ctx: DbContext = provider.get_owned()?;

ctx.set::<Blog>();
ctx.ensure_created().await?;       // create tables (dev/test); migrations are preferred in production

ctx.add::<Blog>(Blog {
    blog_id: 0,
    url: "https://example.com".into(),
    rating: 5,
    posts: HasMany::new(),
});
ctx.save_changes().await?;

let expr = linq!(|b: Blog| b.rating >= 4);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;
```

## Features

| Area | Highlights |
|------|-----------|
| Entity mapping | `#[derive(EntityType)]`, 12+ field attributes, navigation types, compile-time registration |
| Query | `linq!` expression trees, filter/join/group_by/aggregation, CTEs, window functions, lazy loading (opt-in) |
| Persistence | `save_changes()`, parameterized SQL, transactions and savepoints, cascade delete, bulk `execute_update` / `execute_delete` |
| Migrations | model diff, Up/Down SQL, migration history, FK `ON DELETE` |
| DI | `add_dbcontext` / `add_dbcontext_keyed` / `add_dbcontext_anykey` / async variants on rust-dix 0.10 (Scoped, owned + shared resolution, typed keys, dynamic contexts) |
| Interceptors | before/after/failure hooks around `save_changes()` |
| Observability | optional `tracing` spans and slow-query warnings |

## Crate layout

| Crate | Purpose |
|-------|---------|
| `rust-ef` | core: `DbContext`, `DbSet`, query builder, change tracking, migrations |
| `rust-ef-macros` | `#[derive(EntityType)]`, `linq!`, `column!`, `#[entity]` |
| `rust-ef-sqlite` / `rust-ef-postgres` / `rust-ef-mysql` | database providers |
| `rust-ef-cli` | migration / scaffold CLI (not published) |

The core is provider- and DI-agnostic: providers inject a factory into
`DbContextOptions`, and `crates/core/src/di.rs` is the only rust-dix integration
surface.

## Core modules

```
entity.rs              IEntityType, IFromRow, IGetKeyValues, IEntitySnapshot
metadata.rs            EntityTypeMeta, PropertyMeta, NavigationMeta
registration.rs        compile-time entity / configuration registration (inventory)
provider/              IDatabaseProvider, ISqlGenerator, IAsyncConnection, DbValue
db_context/            DbContext, DbContextOptions, save pipeline
db_set.rs              IDbSet<T>, DbSet<T>
query/                 IQueryable<T>, QueryBuilder<T>, linq! internals
change_executor/       INSERT / UPDATE / DELETE execution
model_builder.rs       ModelBuilder, IEntityTypeConfiguration<T>
tracking.rs            ChangeTracker (property-level snapshots)
relations.rs           BelongsTo, HasMany, HasOne, DeleteBehavior
navigation_loader.rs   eager loading (Include / ThenInclude)
lazy.rs                opt-in lazy navigation loading
cascade.rs             cascade save / delete
transaction.rs         context-scoped transactions and savepoints
interceptor.rs         ISaveChangesInterceptor pipeline
migration/             MigrationEngine, diffing, history
di.rs                  rust-dix 0.10 integration
```

## Cargo features

| Feature | Enables |
|---------|---------|
| `serde` | Serde impls for `DbValue` |
| `chrono` | `DateTime<Utc>` / `NaiveDateTime` / `NaiveDate` mapping |
| `uuid` | `uuid::Uuid` mapping |
| `decimal` | `rust_decimal::Decimal` mapping |
| `tracing` | query spans and slow-query warnings |

## Documentation

- Handbook: <https://rf2026.github.io/rust-ef/>
- API docs: <https://docs.rs/rust-ef>
- Examples: [`examples/blog`](https://github.com/Rust-Framework/rust-ef/tree/main/examples/blog), [`examples/soft_delete`](https://github.com/Rust-Framework/rust-ef/tree/main/examples/soft_delete), [`examples/audit`](https://github.com/Rust-Framework/rust-ef/tree/main/examples/audit)

## License

MIT — see [LICENSE](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE).
