# Rust Entity Framework (rust-ef)

[![Crates.io](https://img.shields.io/crates/v/rust-ef)](https://crates.io/crates/rust-ef)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Documentation](https://img.shields.io/badge/docs-mdBook-blue.svg)](https://rf2026.github.io/rust-ef/)

EFCore-inspired ORM for Rust — `DbContext` / `DbSet<T>` / `IEntityType` with rust-dicore DI integration.

**[在线文档](https://rf2026.github.io/rust-ef/)** ?? mdBook 构建的完整开发者手�?

---

## Quick Start

```toml
[dependencies]
rust-ef = "1.3"
rust-ef-sqlite = "1.3"
rust-dicore = "0.5.1"
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

### Fluent Configuration (auto-discovered)

`#[derive(EntityType)]` auto-registers entities at compile time. `DbContext::from_options()` automatically discovers all registered entities and applies `#[entity(T)]` configurations �?no manual `discover_entities()` call needed.

```rust
#[derive(Default)]
pub struct BlogConfig;

#[entity(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.to_table("blogs_v2");
        entity.property_named("url").has_column_name("blog_url");
        entity.has_data(vec![
            Blog { blog_id: 1, url: "https://example.com".into(), rating: 5,
                   posts: HasMany::default() },
        ]);
    }
}
```

### DI Registration + Usage (Single DB)

```rust
use rust_dicore::*;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Register
    let provider = ServiceCollection::new()
        .add_dbcontext(|options| {
            options.use_sqlite("data source=app.db");
        })
        .build()
        .unwrap();

    // 2. Resolve as owned DbContext (&mut self access, no locks)
    let mut ctx: DbContext = provider.get_owned();

    ctx.save_changes().await?;
    Ok(())
}
```

### Multi-DB (Keyed Registration + Entity Isolation)

Tag entities with `#[context("key")]` to isolate them per keyed `DbContext`. `#[entity(T, "key")]` applies configurations to the matching context only.

```rust
// Default context entity �?no #[context] attribute
#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog { /* ... */ }

// Keyed context entity �?tagged for "logs" context
#[derive(Debug, Clone, EntityType)]
#[context("logs")]
#[table("log_entries")]
pub struct LogEntry { /* ... */ }

// Register two keyed DbContexts
let provider = ServiceCollection::new()
    .add_dbcontext_keyed("primary", |options| {
        options.use_postgres("host=primary/db");
    })
    .add_dbcontext_keyed("logs", |options| {
        options.use_sqlite("logs.db");
    })
    .build()
    .unwrap();

let primary: Arc<DbContext> = provider.get_keyed("primary");
let logs: Arc<DbContext> = provider.get_keyed("logs");
// primary manages Blog; logs manages LogEntry �?isolated by context_key
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
.add_dbcontext(|options| {
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

### Navigation (Lazy Loading, opt-in since v1.1)

```rust
// Enable lazy loading at the options level
let mut options = DbContextOptionsBuilder::new();
options.use_sqlite("app.db").use_lazy_loading(true);
let mut ctx = DbContext::from_options(&options.build())?;

let blogs = ctx.set::<Blog>().query().to_list().await?;
for blog in &blogs {
    // Navigation loaded on first access; subsequent reads hit cache
    let posts = blog.posts.load().await?;
    println!("{}: {} posts", blog.url, posts.len());
}
```

> Lazy Loading is **opt-in** (`use_lazy_loading(true)`, default `false`). When disabled, use `linq!(...; include ...)` for eager loading.

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

### Attach �?Modify �?SaveChanges

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
    .add_dbcontext_keyed("read", |o| o.use_postgres("host=replica/db"))
    .add_dbcontext_keyed("write", |o| o.use_postgres("host=primary/db"))
    .build()
    .unwrap();

let read: Arc<DbContext> = provider.get_keyed("read");
let write: Arc<DbContext> = provider.get_keyed("write");
```

---

## Web Application Integration

`DbContext` is registered as **Scoped** via `add_dbcontext` �?each request gets its own
instance (unit-of-work isolation). No locks needed.

```rust
use std::sync::Arc;
use rust_ef::db_context::DbContext;
use rust_ef::di::*;

// Register as Scoped (�?ASP.NET Core AddDbContext<T>)
let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

// Each request creates a scope. Handlers own a fresh DbContext via get_owned().
let mut ctx: DbContext = provider.get_owned();

// Inject into handlers via DI — bare T field marked #[inject(owned)] → owned resolution
#[derive(Inject)]
pub struct MyHandler {
    #[inject(owned)]
    ctx: DbContext,
}

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<MyRequest, MyResponse> for MyHandler {
    async fn handle(&mut self, req: MyRequest) -> Result<MyResponse> {
        self.ctx.set::<Blog>().add(blog);
        self.ctx.save_changes().await?;
        // ...
    }
}
```

> **`Arc<Mutex<DbContext>>` is an anti-pattern**: it causes cross-request tracking
> pollution — Thread A's `save_changes()` would commit Thread B's pending changes.
> Use owned resolution (`get_owned()`) or Scoped lifecycle instead, aligned with EFCore design.

### Recommended patterns

```rust
// �?Step-by-step let bindings �?readable and debuggable
let set = ctx.set::<Blog>();
let expr = linq!(|b: Blog| b.slug == req.slug);
let exists = set.filter(expr).first_or_default().await?;

// �?linq! expression binding �?filter logic independently named
let expr = linq!(|b: Blog| b.rating > 3);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;

// �?Create flow: check �?insert �?save �?re-query by PK (for navigation)
let mut blog = req.to_entity(uid, now);
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
// blog.id is now populated �?no need to re-query just for the ID

// Only re-query if you need navigation properties, and always by PRIMARY KEY
let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
    include b.category;
).first_or_default().await?;
```

---

## Common Pitfalls & Anti-Patterns

### Don't re-query just for the auto-increment ID

```rust
// �?WRONG: id is already on the entity
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q).first_or_default().await?;
let id = saved.unwrap().id;

// �?CORRECT: use the entity directly
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
let id = blog.id; // already populated!
```

### Don't use string-based column names

```rust
// �?WRONG: no compile-time checking
ctx.set::<Blog>().query().filter_column("slug", "=", "hello").to_list().await?;

// �?CORRECT: type-safe linq! expressions
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == "hello").to_list().await?;
```

### Don't repeat `is_deleted` in every query �?use global query filters

```rust
// �?WRONG: repetitive, easy to forget
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q && !b.is_deleted)

// �?CORRECT: register once at startup
ctx.model().entity::<Blog>()
    .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));
// All queries now automatically exclude deleted records

// Admin queries that need to see all records:
ctx.set::<Blog>().query_ignore_filters().to_list().await?;
```

### Don't use `Arc<Mutex<DbContext>>` — use owned resolution

```rust
// ❌ WRONG: cross-request tracking pollution
#[derive(Inject)]
pub struct MyHandler {
    ctx: Arc<Mutex<DbContext>>,
}

// ✅ CORRECT: Scoped registration + owned resolution, each request gets its own instance
// main.rs:
.add_dbcontext(|o| o.use_sqlite("app.db"));
// handler:
#[derive(Inject)]
pub struct MyHandler {
    #[inject(owned)]
    ctx: DbContext,  // bare T + #[inject(owned)] → get_owned()
}

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<MyRequest, MyResponse> for MyHandler {
    async fn handle(&mut self, req: MyRequest) -> Result<MyResponse> {
        self.ctx.set::<Blog>().add(blog);
        self.ctx.save_changes().await?;
        // ...
    }
}
```

### Prefer `detect_changes()` over `update()` for modifications

```rust
// �?LESS PRECISE: update() marks the entire entity as Modified
ctx.set::<Blog>().update(blog);
ctx.save_changes().await?;

// �?BETTER: detect_changes() only marks actually changed fields
blog.is_deleted = true;
ctx.set::<Blog>().detect_changes();
ctx.save_changes().await?;
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
    ??? rust-dicore (crates.io ??DI, resolves Arc<DbContext>)
    ??? rust-ef (ORM, workspace: crates/core)
          DbContext (type-map set storage, no entity-specific fields)
          ??? DbContext       ??concrete session/unit-of-work type
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

DbContext (concrete context type)
    ??? provider() ??&dyn IDatabaseProvider
    ??? save_changes() ??SaveChangesResult
    ??? change_tracker() ??&ChangeTracker

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
| `provider_factory` in options | Provider extensions inject factory closures; core stays decoupled |
| `SetOps<T>` dispatchers | Type-erased `save_changes()` iterates all entity types |
| Keyed registration for multi-DB | `add_dbcontext_keyed` + `provider.get_keyed()` |
| Interceptor pipeline | `options.add_interceptor(...)` for cross-cutting concerns |

---

## Features

| Category | Feature |
|----------|---------|
| **Entity** | `#[derive(EntityType)]` with 13 attributes, navigation types, auto-discovery |
| **Query** | `linq!` expression trees, `filter` / `filter_column`, join, group_by, aggregation, IN/NOT IN subqueries |
| **Advanced Query** | CTE (`linq!(with ...)` syntax sugar), Window functions (10 kinds), Lazy Loading (opt-in) |
| **Persistence** | `save_changes()`, parameterized queries, transactions |
| **DI** | `add_dbcontext` / `add_dbcontext_keyed` / `add_dbcontext_from_options`, `Arc<DbContext>`, multi-DB context key isolation |
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
| `#[context("key")]` | Multi-DB context key isolation (v1.1) |

---

## License

MIT
