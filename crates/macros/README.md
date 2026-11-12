# rust-ef-macros

[![crates.io](https://img.shields.io/crates/v/rust-ef-macros.svg)](https://crates.io/crates/rust-ef-macros)
[![docs.rs](https://docs.rs/rust-ef-macros/badge.svg)](https://docs.rs/rust-ef-macros)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE)

Procedural macros for [`rust-ef`](https://crates.io/crates/rust-ef), an
EF Core-inspired ORM for Rust. Re-exported by `rust-ef`, so applications normally
never depend on this crate directly.

## What's included

| Macro | Purpose |
|-------|---------|
| `#[derive(EntityType)]` | Generates `IEntityType`, `IFromRow`, `IGetKeyValues` and `IEntitySnapshot`; registers the entity for compile-time discovery |
| `linq!(...)` | Typed query DSL: `linq!(\|b: T\| ...)` predicates, `order_by`, `skip`/`take`, `include`, joins, aggregation, CTEs, window functions, bulk `execute_update` / `execute_delete` |
| `column!(Entity::field)` | Resolves an entity field to its database column name at compile time |
| `#[entity(Entity)]` / `#[entity(Entity, "context_key")]` | Applies an `IEntityTypeConfiguration<T>` implementation to the model (optionally per keyed context) |
| `#[derive(DbContextKey)]` | Typed keyed-`DbContext` key: implements `IDbContextKey` and the rust-dix `From<Key> for ServiceKey` contract (for `#[inject(owned, key = Key::Read)]` and keyed registration) |

## Entity attributes

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
    #[unique]
    pub url: String,

    pub rating: i32,

    #[navigation]
    pub posts: HasMany<Post>,
}
```

| Attribute | Meaning |
|-----------|---------|
| `#[table("...")]` | explicit table name |
| `#[primary_key]` | primary key column |
| `#[auto_increment]` | auto-increment / identity |
| `#[required]` | `NOT NULL` |
| `#[max_length(n)]` | max string length |
| `#[column("db_col")]` | different database column name |
| `#[foreign_key(Entity)]` | foreign-key target |
| `#[navigation]` | navigation property (`BelongsTo` / `HasMany` / `HasOne`) |
| `#[not_mapped]` | exclude from persistence |
| `#[index]` / `#[unique]` | create an index / unique index |
| `#[concurrency_check]` | optimistic concurrency token |
| `#[context("key")]` | entity belongs to a keyed `DbContext` |
| `#[on_delete(...)]` | FK delete behavior (also configurable via Fluent API) |

## Configuration blocks

```rust
#[derive(Default)]
pub struct BlogConfig;

#[entity(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.property_named("url").has_column_name("blog_url");
        entity.has_index(linq!(index |b: Blog| b.rating));
    }
}
```

`#[entity(T, "key")]` scopes the configuration to a keyed context.

## Typed DbContext keys

```rust
use rust_ef::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, DbContextKey)]
enum Db {
    Read,
    Write,
    #[key("audit-log")]
    Audit,
}
```

The derive generates `IDbContextKey::name()` (snake-case variant name, or the
`#[key("...")]` override) and `From<Db> for rust_dix::ServiceKey`. That makes
`add_dbcontext_keyed(Db::Write, ...)`, `provider.get_owned_context(Db::Write)`
and `#[inject(owned, key = Db::Write)]` share one typed key. The enum must derive
`Debug + Clone + Copy + PartialEq + Eq + Hash`.

## MSRV

Rust **1.95**.

## License

MIT — see [LICENSE](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE).
