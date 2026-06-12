# Rust Entity Framework (lref)

[![Crates.io](https://img.shields.io/crates/v/lref)](https://crates.io/crates/lref)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Interface-oriented, EFCore-inspired ORM for Rust — `IDbContext` / `IDbSet<T>` / `IEntityType` with lrdi DI integration.

---

## Quick Start

```toml
[dependencies]
lref = "0.3"
lref-provider-sqlite = "0.3"
lrdi = "0.2"
tokio = { version = "1", features = ["full"] }
```

### Define Entities

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

### DI Registration + Usage

```rust
use lrdi::ServiceCollection;
use lref::di::*;
use lref::db_context::DbContext;
use lref_provider_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Register
    let provider = ServiceCollection::new()
        .add_dbcontext::<DbContext>(|options| {
            options.use_sqlite("data source=app.db");
            // or: options.use_sqlite_in_memory();
            // or: options.use_postgres("host=localhost dbname=app");
            // or: options.use_mysql("mysql://user:pass@localhost/db");
        })
        .build()
        .unwrap();

    // 2. Resolve as interface
    let ctx: Arc<dyn IDbContext> = provider.get();

    // 3. Use via concrete methods (call set<T> on the concrete type)
    // Or hold a typed reference:
    // let mut ctx = DbContext::from_options(&options)?;
    // ctx.set::<Blog>().add(Blog { blog_id: 0, url: "https://example.com".into(), rating: 5, posts: HasMany::new() });
    // ctx.set::<Post>().add(Post { post_id: 0, title: "Hello".into(), content: None, blog_id: 1, blog: BelongsTo::new() });

    ctx.save_changes().await?;

    Ok(())
}
```

---

## Architecture

```
User Application
    ├── lrdi (DI container — resolves Arc<dyn IDbContext>)
    └── lref (ORM)
          DbContext (type-map set storage, no entity-specific fields)
          ├── IDbContext     — object-safe session trait
          ├── IDbSet<T>      — entity collection (mutation)
          ├── IQueryable<T>  — query entry point
          └── IDatabaseProvider — backend abstraction
                ├── lref-provider-sqlite    (use_sqlite: injects factory)
                ├── lref-provider-postgres  (use_postgres: injects factory)
                └── lref-provider-mysql     (use_mysql: tag only)
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

---

## Features

| Category | Feature |
|----------|---------|
| **Entity** | `#[derive(EntityType)]` with 12 attributes, navigation types |
| **Query** | LINQ-style `QueryBuilder`: filter, join, group_by, aggregation, bulk ops |
| **Persistence** | `save_changes_all!` macro, parameterized queries, transactions |
| **DI** | `add_dbcontext<DbContext>(|o| o.use_sqlite(...))`, `Arc<dyn IDbContext>` |
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
