# lref �?Core ORM Crate

[![Crates.io](https://img.shields.io/crates/v/lref)](https://crates.io/crates/lref)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](../../LICENSE)

EFCore-inspired ORM core: traits, query builder, change tracking, migration engine, DI integration.

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
use rust_dicore::*;
use lref::di::*;
use lref_provider_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|o| o.use_sqlite("app.db"))
    .build().unwrap();

let mut ctx: DbContext = provider.get_owned();
```

---

## Module Map

```
lref/src/
├── entity.rs       �?IEntityType, IFromRow, IGetKeyValues, IEntitySnapshot
├── metadata.rs     �?EntityTypeMeta, PropertyMeta, NavigationMeta
├── provider.rs     �?IDatabaseProvider, ISqlGenerator, IAsyncConnection, DbValue
├── db_context.rs   �?DbContext, DbContextOptions
├── db_set.rs       �?IDbSet<T>, DbSet<T>
├── query.rs        �?IQueryable<T>, QueryBuilder<T>
├── change_executor.rs �?ChangeExecutor (INSERT/UPDATE/DELETE)
├── model_builder.rs   �?ModelBuilder, IEntityTypeConfiguration<T>
├── tracking.rs     �?ChangeTracker (property-level snapshots)
├── relations.rs    �?BelongsTo, HasMany, HasOne (no trait bounds)
├── migration.rs    �?MigrationEngine
├── di.rs           ?rust-dicore integration (add_dbcontext / add_dbcontext_keyed)
├── cache.rs        �?DbCache (Identity Map)
└── error.rs        �?EFError, EFResult
```

---

## Core Traits

### Entity

```rust
pub trait IEntityType: Send + Sync + 'static { fn entity_meta() -> EntityTypeMeta; }
pub trait IFromRow: IEntityType + Sized { fn from_row(v: &[String]) -> EFResult<Self>; }
pub trait IGetKeyValues: IEntityType { fn key_values(&self) -> HashMap<String, DbValue>; }
pub trait IEntitySnapshot: IEntityType { fn snapshot(&self) -> HashMap<String, DbValue>; }
```

### Session (concrete context type)

```rust
pub struct DbContext {
    // type-map set storage, no entity-specific fields
}

impl DbContext {
    pub fn provider(&self) -> &dyn IDatabaseProvider;
    pub fn change_tracker_mut(&mut self) -> &mut ChangeTracker;
    pub fn change_tracker(&self) -> &ChangeTracker;
    pub async fn save_changes(&mut self) -> EFResult<SaveChangesResult>;
    pub async fn use_transaction<F, Fut, R>(&self, f: F) -> EFResult<R>;
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
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64>;
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>>;
    async fn begin_transaction(&mut self) -> EFResult<()>;
    async fn commit_transaction(&mut self) -> EFResult<()>;
    async fn rollback_transaction(&mut self) -> EFResult<()>;
}

#[async_trait]
pub trait IDatabaseProvider: Send + Sync {
    fn sql_generator(&self) -> Box<dyn ISqlGenerator>;
    async fn get_connection(&self) -> EFResult<Box<dyn IAsyncConnection>>;
    async fn execute_migration_command(&self, sql: &str) -> EFResult<()>;
    fn name(&self) -> &str;
}
```

---

## DI Integration

```rust
use rust_dicore::*;
use lref::di::*;
use lref_provider_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|o| o.use_sqlite("app.db"))
    .build().unwrap();

// Owned (recommended): &mut self access, no locks
let mut ctx: DbContext = provider.get_owned();

// Shared (within a scope): Arc<DbContext>, &self only
// let scope = provider.create_scope();
// let ctx: Arc<DbContext> = scope.get();
```

**Provider factory**: `use_sqlite()` injects a closure into `DbContextOptions`. `DbContext::from_options()` calls it to create the provider �?core stays fully decoupled.

---

## QueryBuilder API

All query operations go through the `linq!` macro �?the string-based APIs (`include_named` / `order_by("col")` / `sum("col")` / `find_by_id` etc.) have been removed. The macro expands to `#[doc(hidden)]` `*_internal` methods at compile time.

```rust
// Form A: filter closure (reusable BoolExpr or direct query)
let expr = linq!(|b: Blog| b.rating > 5);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;

// Form B: multi-clause query (filter + clauses separated by `;`)
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.published;
    include b.posts then b.comments;
    order_by b.created_at desc;
).to_list().await?;

// Aggregates (terminals)
let total: f64 = linq!(ctx.set::<Blog>(); sum b.views).await?;
let top: i32 = linq!(ctx.set::<Blog>(); max b.rating).await?.unwrap_or(0);
let n: i64 = linq!(ctx.set::<Blog>(); count).await?;

// JOIN via multi-param closure
let rows = linq!(ctx.set::<Blog>();
    inner_join |a: Blog, b: Post| a.blog_id == b.blog_id
).to_list().await?;

// Bulk update
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.rating < 0.1;
    set b.published, false;
    execute_update
).await?;

// Bulk delete
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.rating < 1)
    .execute_delete().await?;

// Form C: value-producing (for ModelBuilder)
builder.has_query_filter(linq!(filter |b: Blog| b.deleted_at.is_null()));
builder.has_index(linq!(index |b: Blog| (b.author_id, b.created_at)));
builder.has_key(linq!(key |b: Blog| b.blog_id));

// Terminals on QueryBuilder<T>
ctx.set::<Blog>().query().find(1).await?            // Option<T> (PK metadata, no hardcoded "id")
ctx.set::<Blog>().query().first().await?           // T (errors if empty)
ctx.set::<Blog>().query().first_or_default().await?// Option<T>
ctx.set::<Blog>().query().last().await?            // T (PK desc when no explicit order)
ctx.set::<Blog>().query().last_or_default().await? // Option<T>
ctx.set::<Blog>().query().single().await?          // T (errors if !=1)
ctx.set::<Blog>().query().single_or_default().await? // Option<T> (errors if >1)
ctx.set::<Blog>().query().count().await?           // i64
ctx.set::<Blog>().query().long_count().await?     // i64 (alias)
ctx.set::<Blog>().query().any().await?             // bool
ctx.set::<Blog>().query().all(|b| b.published).await?  // bool (Rust-side predicate)
ctx.set::<Blog>().query().contains(1).await?      // bool (PK existence)
ctx.set::<Blog>().query().distinct().to_list().await?   // SELECT DISTINCT
```

## License

MIT
