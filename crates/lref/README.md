# lref — Core ORM Crate

[![Crates.io](https://img.shields.io/crates/v/lref)](https://crates.io/crates/lref)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

Interface-oriented ORM core: traits, query builder, change tracking, migration engine, DI integration.

---

## Quick Example

```rust
use lref::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key] #[auto_increment] pub blog_id: i32,
    #[required] #[max_length(200)] pub url: String,
    pub rating: i32,
    #[navigation] pub posts: HasMany<Post>,
}

// DI registration
use lrdi::ServiceCollection;
use lref::di::*;
use lref_provider_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|o| o.use_sqlite("app.db"))
    .build().unwrap();

let ctx: Arc<dyn IDbContext> = provider.get();
```

---

## Module Map

```
lref/src/
├── entity.rs       — IEntityType, IFromRow, IGetKeyValues, IEntitySnapshot
├── metadata.rs     — EntityTypeMeta, PropertyMeta, NavigationMeta
├── provider.rs     — IDatabaseProvider, ISqlGenerator, IAsyncConnection, DbValue
├── db_context.rs   — IDbContext, IDbContextExt, DbContext, DbContextOptions
├── db_set.rs       — IDbSet<T>, DbSet<T>
├── query.rs        — IQueryable<T>, QueryBuilder<T>
├── change_executor.rs — ChangeExecutor (INSERT/UPDATE/DELETE)
├── model_builder.rs   — ModelBuilder, IEntityTypeConfiguration<T>
├── tracking.rs     — ChangeTracker (property-level snapshots)
├── relations.rs    — BelongsTo, HasMany, HasOne (no trait bounds)
├── migration.rs    — MigrationEngine
├── di.rs           — lrdi integration (add_dbcontext / FromDbContextOptions)
├── cache.rs        — DbCache (Identity Map)
└── error.rs        — LrefError, LrefResult
```

---

## Core Traits

### Entity

```rust
pub trait IEntityType: Send + Sync + 'static { fn entity_meta() -> EntityTypeMeta; }
pub trait IFromRow: IEntityType + Sized { fn from_row(v: &[String]) -> LrefResult<Self>; }
pub trait IGetKeyValues: IEntityType { fn key_values(&self) -> HashMap<String, DbValue>; }
pub trait IEntitySnapshot: IEntityType { fn snapshot(&self) -> HashMap<String, DbValue>; }
```

### Session (object-safe)

```rust
#[async_trait]
pub trait IDbContext: Send + Sync {
    fn provider(&self) -> &dyn IDatabaseProvider;
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker;
    fn change_tracker(&self) -> &ChangeTracker;
    async fn save_changes(&mut self) -> LrefResult<SaveChangesResult>;
}

#[async_trait]
pub trait IDbContextExt: IDbContext {
    async fn use_transaction<F, Fut, R>(&self, f: F) -> LrefResult<R>;
}
```

### Collection

```rust
pub trait IQueryable<T: IEntityType> { fn query(&self) -> QueryBuilder<T>; }

pub trait IDbSet<T: IEntityType>: IQueryable<T> + Send + Sync {
    fn add(&mut self, entity: T);
    fn remove_all(&mut self);
    fn added_entities(&self) -> Vec<&T>;
    fn modified_entities(&self) -> Vec<&T>;
    fn deleted_entities(&self) -> Vec<&T>;
    fn clear_entries(&mut self);
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}
```

### Provider

```rust
pub trait ISqlGenerator: Send + Sync { /* select, insert, update, delete, ... */ }

#[async_trait]
pub trait IAsyncConnection: Send + Sync {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<u64>;
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> LrefResult<Vec<Vec<String>>>;
    async fn begin_transaction(&mut self) -> LrefResult<()>;
    async fn commit_transaction(&mut self) -> LrefResult<()>;
    async fn rollback_transaction(&mut self) -> LrefResult<()>;
}

#[async_trait]
pub trait IDatabaseProvider: Send + Sync {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator>;
    async fn get_connection(&self) -> LrefResult<Box<dyn IAsyncConnection>>;
    async fn execute_migration_command(&self, sql: &str) -> LrefResult<()>;
    fn name(&self) -> &str;
}
```

---

## DI Integration

```rust
use lrdi::ServiceCollection;
use lref::di::*;
use lref_provider_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|o| o.use_sqlite("app.db"))
    .build().unwrap();

let ctx: Arc<dyn IDbContext> = provider.get();
```

**Provider factory**: `use_sqlite()` injects a closure into `DbContextOptions`. `DbContext::from_options()` calls it to create the provider — core stays fully decoupled.

---

## QueryBuilder API

```rust
// Filtering
query.filter_column("col", "=", value).filter_in("col", vec![1,2,3])
     .filter_is_null("col").filter_is_not_null("col").filter_between("col", low, high)

// Ordering, pagination, JOIN, grouping
     .order_by_column("col").order_by_desc_column("col").skip(10).take(20)
     .inner_join("t2", "a", "b").left_join("t2", "a", "b")
     .group_by(&["col"]).having("COUNT(*) > 1")

// Eager loading
     .include_named("posts")

// Terminal
     .to_list().await?        // Vec<T>
     .first().await?          // T
     .first_or_default().await?  // Option<T>
     .count().await?          // i64
     .any().await?            // bool
     .sum("col").await?       // f64
     .avg("col").await?       // f64

// Bulk
     .execute_update().set_column("col", value).execute().await?
     .execute_delete().await?
```

## License

MIT
