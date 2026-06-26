# Rust Entity Framework (rust-ef)

[![Crates.io](https://img.shields.io/crates/v/rust-ef)](https://crates.io/crates/rust-ef)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Documentation](https://img.shields.io/badge/docs-mdBook-blue.svg)](https://rf2026.github.io/rust-ef/)

Interface-oriented, EFCore-inspired ORM for Rust ??`IDbContext` / `IDbSet<T>` / `IEntityType` with rust-dicore DI integration.

**[在线文档](https://rf2026.github.io/rust-ef/)** ?? mdBook 构建的完整开发者手册

---

## Quick Start

```toml
[dependencies]
rust-ef = "1.0"
rust-ef-sqlite = "1.0"
rust-dicore = "0.2"
tokio = { version = "1", features = ["full"] }
```

### Define Entities

```rust
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key] #[auto_increment] pub blog_id: i32,
    #[required] #[max_length(200)] pub url: String,
    pub rating: i32,
    #[navigation] pub posts: HasMany<Post>,
}

#[derive(Debug, Clone, EntityType)]
#[table("posts")]
pub struct Post {
    #[primary_key] #[auto_increment] pub post_id: i32,
    #[required] #[max_length(200)] pub title: String,
    pub content: Option<String>,
    #[foreign_key(Blog)] pub blog_id: i32,
    #[navigation] pub blog: BelongsTo<Blog>,
}
```

### DI Registration + Usage (Single DB)

```rust
use rust_dicore::ServiceCollection;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Register
    let provider = ServiceCollection::new()
        .add_dbcontext::<DbContext>(|options| {
            options.use_sqlite("data source=app.db");
        })
        .build()
        .unwrap();

    // 2. Resolve as interface
    let ctx: Arc<dyn IDbContext> = provider.get();

    ctx.save_changes().await?;
    Ok(())
}
```

### Multi-DB (Keyed Registration)

```rust
let provider = ServiceCollection::new()
    .add_dbcontext_keyed::<DbContext>("primary", |options| {
        options.use_postgres("host=primary/db");
    })
    .add_dbcontext_keyed::<DbContext>("logs", |options| {
        options.use_sqlite("logs.db");
    })
    .build()
    .unwrap();

let primary: Arc<dyn IDbContext> = provider.get_keyed("primary");
let logs: Arc<dyn IDbContext> = provider.get_keyed("logs");
```

### SaveChanges Interceptors

```rust
use rust_ef::interceptor::{ISaveChangesInterceptor, SaveChangesContext};

struct AuditInterceptor;
#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        tracing::info!("Saving +{} ~{} -{}", ctx.added_count(), ctx.modified_count(), ctx.deleted_count());
        Ok(())
    }
}

// Register
.add_dbcontext::<DbContext>(|options| {
    options
        .use_sqlite("app.db")
        .add_interceptor(AuditInterceptor);
})
```

---

## Best Practices Guide

The following patterns are recommended for production use. They emphasize **readability** and **clarity**: split complex operations into named `let` bindings rather than squeezing everything into one chain.

### Recommended Query Style

```rust
let set = ctx.set::<Blog>();

let expr = linq!(|b: Blog| b.rating > 0.5);

return set.filter(expr).to_list().await?;
```

This is clearer than all-in-one chaining because each step has a name: the data source (`set`), the filter logic (`expr`), and the execution (`to_list`).

### Filtering & Sorting

```rust
let posts = linq!(ctx.set::<Post>(), |p: Post| p.blog_id == target_id && p.title.contains("Rust");
    order_by p.created_at desc;
    skip offset;
    take page_size;
).to_list().await?;
```

### Reusable LINQ Expressions

```rust
let min_rating = 4;
let high_rated = linq!(|b: Blog| b.rating > min_rating);

let blogs = ctx.set::<Blog>().filter(high_rated).to_list().await?;
let count = ctx.set::<Blog>().filter(high_rated).count().await?;
```

### Navigation (Eager Loading)

```rust
let blogs = linq!(ctx.set::<Blog>(); include b.posts then b.comments)
    .to_list()
    .await?;
```

> There is **no Lazy Loading**. Always use `linq!(...; include ...)` when you need related data.

### Bulk Update

```rust
let affected = linq!(
    ctx.set::<Blog>(), |b: Blog| b.rating < 3;
    set b.rating, 3;
    execute_update
).await?;
```

### Bulk Delete

```rust
let affected = ctx
    .set::<Blog>()
    .query()
    .filter(linq!(|b: Blog| b.rating < 1))
    .execute_delete()
    .await?;
```

### Attach → Modify → SaveChanges

```rust
let mut blog = ctx.set::<Blog>().query().find(1).await?.unwrap();
blog.rating = 10;

ctx.set::<Blog>().update(blog);
ctx.save_changes().await?;
```

### Global Query Filter (Soft Delete)

```rust
ctx.model().entity::<Blog>().has_query_filter(linq!(filter |b: Blog| b.deleted_at.is_null()));
ctx.set::<Blog>();
// All subsequent queries automatically append the filter expression
```

### Multi-DB (Keyed)

```rust
let provider = ServiceCollection::new()
    .add_dbcontext_keyed::<DbContext>("read", |o| o.use_postgres("host=replica/db"))
    .add_dbcontext_keyed::<DbContext>("write", |o| o.use_postgres("host=primary/db"))
    .build()
    .unwrap();

let read: Arc<dyn IDbContext> = provider.get_keyed("read");
let write: Arc<dyn IDbContext> = provider.get_keyed("write");
```

---

## Full Documentation

See [`docs/rust-ef/INDEX.md`](docs/rust-ef/INDEX.md) for the complete best-practices book covering:

- Entity design with `#[derive(EntityType)]`
- One-to-many and many-to-many relationships
- Advanced queries: aggregation, GROUP BY, JOIN, raw SQL
- Change tracking: Add / Attach / Update / Remove
- Bulk operations, transactions, and migrations
- DI integration, interceptors, and multi-database patterns
- Common pitfalls, performance tips, and code-review checklist

---

## Architecture

```
User Application
    ??? rust-dicore (crates.io ??DI, resolves Arc<dyn IDbContext>)
    ??? rust-ef (ORM, workspace: crates/core)
          DbContext (type-map set storage, no entity-specific fields)
          ??? IDbContext     ??object-safe session trait
          ??? IDbSet<T>      ??entity collection (mutation)
          ??? IQueryable<T>  ??query entry point
          ??? ISaveChangesInterceptor ??before/after save hooks
          ??? IDatabaseProvider ??backend abstraction
                ??? crates/sqlite    ??rust-ef-sqlite  (use_sqlite)
                ??? crates/postgres  ??rust-ef-postgres (use_postgres)
                ??? crates/mysql     ??rust-ef-mysql   (use_mysql)
```

### Interface Hierarchy

```
IEntityType ??? IFromRow
             ??? IGetKeyValues
             ??? IEntitySnapshot

IQueryable<T> ??? IDbSet<T>

IDbContext (object-safe ??dyn compatible)
    ??? provider() ??&dyn IDatabaseProvider
    ??? save_changes() ??SaveChangesResult
    ??? change_tracker() ??&ChangeTracker

IDbContextExt (non-object-safe ??generic helpers)
    ??? use_transaction(f)

IDatabaseProvider
    ??? sql_generator() ??ISqlGenerator
    ??? get_connection() ??IAsyncConnection
    ??? execute_migration_command(sql)

ISaveChangesInterceptor
    ??? on_saving(ctx)           // pre-commit; Err aborts save
    ??? on_saved(ctx, result)    // post-commit
    ??? on_save_failed(ctx, err) // on error (after rollback)

FromDbContextOptions (DI bridge)
    ??? from_options(&DbContextOptions) ??Self
```

---

## Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| No `DbSet<Blog>` struct fields | `DbContext` uses type-map; sets lazy-created via `set::<T>()` |
| `IDbContext` is object-safe | Enables `Arc<dyn IDbContext>` DI resolution |
| `provider_factory` in options | Provider extensions inject factory closures; core stays decoupled |
| `SetOps<T>` dispatchers | Type-erased `save_changes()` iterates all entity types |
| Generic methods on `IDbContextExt` | Keeps core trait object-safe |
| Keyed registration for multi-DB | `add_dbcontext_keyed` + `provider.get_keyed()` |
| Interceptor pipeline | `options.add_interceptor(...)` for cross-cutting concerns |

---

## Features

| Category | Feature |
|----------|---------|
| **Entity** | `#[derive(EntityType)]` with 12 attributes, navigation types |
| **Query** | `linq!` expression trees, `filter` / `filter_column`, join, group_by, aggregation |
| **Persistence** | `save_changes()`, parameterized queries, transactions |
| **DI** | `add_dbcontext` / `add_dbcontext_keyed` / `add_dbcontext_from_options`, `Arc<dyn IDbContext>` |
| **Interception** | `ISaveChangesInterceptor` ??on_saving/on_saved/on_save_failed hooks |
| **Migrations** | Model diff, Up/Down SQL, history tracking, `MigrationStore` |
| **CLI** | `rust-ef-cli`: `migration init/add/apply/revert/list/script`, `scaffold dbcontext` |

---

## Derive Attributes

| Attribute | EFCore Equivalent |
|-----------|-------------------|
| `#[table]` | `[Table]` |
| `#[primary_key]` | `[Key]` |
| `#[auto_increment]` | convention |
| `#[required]` | `[Required]` |
| `#[max_length]` | `[MaxLength]` |
| `#[column]` | `[Column]` |
| `#[foreign_key]` | `[ForeignKey]` |
| `#[navigation]` | implicit |
| `#[not_mapped]` | `[NotMapped]` |
| `#[index]` / `#[unique]` | `[Index]` |
| `#[concurrency_check]` | `[ConcurrencyCheck]` |

---

## License

MIT
