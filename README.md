# Rust Entity Framework (rust-ef)

[![Crates.io](https://img.shields.io/crates/v/rust-ef)](https://crates.io/crates/rust-ef)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Interface-oriented, EFCore-inspired ORM for Rust — `IDbContext` / `IDbSet<T>` / `IEntityType` with rust-dicore DI integration.

---

## Quick Start

```toml
[dependencies]
rust-ef = "0.3"
rust-ef-sqlite = "0.3"
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
use rust_ef_provider_sqlite::DbContextOptionsBuilderExt as _;

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
    async fn on_saving(&self, ctx: &SaveChangesContext) -> LrefResult<()> {
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

## Architecture

```
User Application
    ├── rust-dicore (DI container — resolves Arc<dyn IDbContext>)
    │     ├── provider.get()           — default registration
    │     └── provider.get_keyed("k")  — keyed registration
    └── rust-ef (ORM)
          DbContext (type-map set storage, no entity-specific fields)
          ├── IDbContext     — object-safe session trait
          ├── IDbSet<T>      — entity collection (mutation)
          ├── IQueryable<T>  — query entry point
          ├── ISaveChangesInterceptor — before/after save hooks
          └── IDatabaseProvider — backend abstraction
                ├── rust-ef-sqlite    (use_sqlite: injects factory)
                ├── rust-ef-postgres  (use_postgres: injects factory)
                └── rust-ef-mysql     (use_mysql: tag only)
```

### Interface Hierarchy

```
IEntityType ─── IFromRow
             ├── IGetKeyValues
             └── IEntitySnapshot

IQueryable<T> ─── IDbSet<T>

IDbContext (object-safe — dyn compatible)
    ├── provider() → &dyn IDatabaseProvider
    ├── save_changes() → SaveChangesResult
    └── change_tracker() → &ChangeTracker

IDbContextExt (non-object-safe — generic helpers)
    └── use_transaction(f)

IDatabaseProvider
    ├── sql_generator() → ISqlGenerator
    ├── get_connection() → IAsyncConnection
    └── execute_migration_command(sql)

ISaveChangesInterceptor
    ├── on_saving(ctx)           // pre-commit; Err aborts save
    ├── on_saved(ctx, result)    // post-commit
    └── on_save_failed(ctx, err) // on error (after rollback)

FromDbContextOptions (DI bridge)
    └── from_options(&DbContextOptions) → Self
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
| **Query** | LINQ-style `QueryBuilder`: filter, join, group_by, aggregation, bulk ops |
| **Persistence** | `save_changes_all!` macro, parameterized queries, transactions |
| **DI** | `add_dbcontext` / `add_dbcontext_keyed` / `add_dbcontext_from_options`, `Arc<dyn IDbContext>` |
| **Interception** | `ISaveChangesInterceptor` — on_saving/on_saved/on_save_failed hooks |
| **Migrations** | Model diff, Up/Down SQL for PostgreSQL/MySQL/SQLite |
| **CLI** | `migration add/apply/revert/list/script`, `scaffold-dbcontext` |

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
