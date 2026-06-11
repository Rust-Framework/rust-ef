# lref

[![Crates.io](https://img.shields.io/crates/v/lref)](https://crates.io/crates/lref)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

Core crate for [Rust Entity Framework](https://crates.io/crates/lref) — an EFCore-inspired ORM for Rust.

Provides the core ORM abstractions: `EntityType`, `DbContext`, `DbSet`, `QueryBuilder`, `ChangeTracker`, `ModelBuilder`, migration engine, and database provider abstraction layer.

---

## Quick Example

```rust
use lref::prelude::*;

// Define entities
#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key] #[auto_increment]
    pub blog_id: i32,
    #[required] #[max_length(200)]
    pub url: String,
    pub rating: i32,
    #[navigation] pub posts: HasMany<Post>,
}

// Build queries
let blogs = db_set.query()
    .filter_column("rating", ">", 3)
    .order_by_column("url")
    .skip(0).take(10)
    .to_list().await?;

// Aggregate
let avg = db_set.query().avg("rating").await?;
let count = db_set.query().count().await?;

// Bulk operations
db_set.query().filter_column("rating", "<", 1)
    .execute_delete().await?;
```

---

## Modules

| Module | Description | EFCore Equivalent |
|--------|-------------|-------------------|
| `entity` | `EntityType` trait, `EntityState`, `FromRow`, `GetKeyValues`, `EntitySnapshot` | Entity classes |
| `db_context` | `DbContext` trait, `SaveChangesResult`, `save_one_set()` | `DbContext` |
| `db_set` | `DbSet<T>` — tracked entity collection | `DbSet<TEntity>` |
| `query` | `QueryBuilder<T>` — LINQ-style fluent queries | `IQueryable<T>` |
| `tracking` | `ChangeTracker` — property-level snapshots & state | `ChangeTracker` |
| `relations` | `BelongsTo<T>`, `HasMany<T>`, `HasOne<T>`, `DeleteBehavior` | Navigation properties |
| `model_builder` | `ModelBuilder`, `EntityTypeBuilder`, `PropertyBuilder` | Fluent API |
| `migration` | `MigrationEngine`, model diff, Up/Down SQL generation | Migration system |
| `provider` | `DatabaseProvider`, `SqlGenerator`, `AsyncConnection`, `DbValue` | Provider abstraction |
| `error` | `LrefError` enum with 12 error variants | Exception hierarchy |
| `cache` | `DbCache` — Identity Map entity cache | — |

---

## Traits

### Core Entity Traits

```rust
pub trait EntityType: Send + Sync + 'static {
    fn entity_meta() -> EntityTypeMeta;
}

pub trait FromRow: EntityType + Sized {
    fn from_row(values: &[String]) -> LrefResult<Self>;
}

pub trait GetKeyValues: EntityType {
    fn key_values(&self) -> HashMap<String, DbValue>;
}

pub trait EntitySnapshot: EntityType {
    fn snapshot(&self) -> HashMap<String, DbValue>;
}
```

### Session / Provider

```rust
#[async_trait]
pub trait DbContext: Send + Sync + Sized {
    type Provider: DatabaseProvider;
    fn provider(&self) -> &Self::Provider;
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker;
    fn change_tracker(&self) -> &ChangeTracker;
    async fn save_changes(&mut self) -> LrefResult<SaveChangesResult>;
}

#[async_trait]
pub trait DatabaseProvider: Send + Sync {
    fn sql_generator(&self) -> Box<dyn SqlGenerator>;
    async fn get_connection(&self) -> LrefResult<Box<dyn AsyncConnection>>;
    async fn execute_migration_command(&self, sql: &str) -> LrefResult<()>;
    fn name(&self) -> &str;
}
```

---

## QueryBuilder API

```rust
// Filtering
query.filter_column("col", "=", value)
     .filter_in("col", vec![1, 2, 3])
     .filter_is_null("col")
     .filter_is_not_null("col")
     .filter_between("col", low, high)

// Ordering & pagination
     .order_by_column("col")
     .order_by_desc_column("col")
     .skip(10).take(20)

// JOIN & grouping
     .inner_join("t2", "col_a", "col_b")
     .left_join("t2", "col_a", "col_b")
     .group_by(&["col"])
     .having("COUNT(*) > 1")

// Include navigation
     .include_named("posts")
     .include_with_join("posts", "posts", "blog_id", "blog_id", "LEFT")

// Terminal (execute)
     .to_list().await?       // Vec<T>
     .first().await?         // T
     .first_or_default().await?  // Option<T>
     .count().await?         // i64
     .any().await?           // bool
     .sum("col").await?      // f64
     .avg("col").await?      // f64
     .min("col").await?      // Option<String>
     .max("col").await?      // Option<String>

// Bulk operations
     .execute_update().set_column("col", value).execute().await?  // u64
     .execute_delete().await?  // u64
```

---

## Database Provider Abstraction

The core crate defines traits. Provider crates implement them:

```rust
// SqlGenerator — dialect-specific SQL
trait SqlGenerator {
    fn select(&self, table: &str, columns: &[&str]) -> String;
    fn insert(&self, table: &str, columns: &[&str], returning: bool) -> String;
    fn update(&self, table: &str, set_columns: &[&str], where_clause: &str) -> String;
    fn delete(&self, table: &str, where_clause: &str) -> String;
    fn parameter_placeholder(&self, index: usize) -> String;
    fn quote_identifier(&self, identifier: &str) -> String;
    // ...
}

// AsyncConnection — typed parameter execution
#[async_trait]
trait AsyncConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<u64>;
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<Vec<Vec<String>>>;
    async fn begin_transaction(&mut self) -> LrefResult<()>;
    async fn commit_transaction(&mut self) -> LrefResult<()>;
    async fn rollback_transaction(&mut self) -> LrefResult<()>;
}
```

## License

MIT — see [LICENSE](../../LICENSE)
