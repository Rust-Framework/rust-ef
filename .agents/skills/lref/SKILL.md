---
name: lref
description: |
  Implement lref (Rust Entity Framework v0.3+) features: define entities with
  #[derive(EntityType)], build DbContext with type-map set storage, register
  via lrdi DI with add_dbcontext<T>(|o| o.use_sqlite("...")), write LINQ-style
  queries, configure migrations, or set up provider extensions. Use when the
  user asks about lref ORM, entity framework, DbContext, or Rust database code.
---

# lref ??Rust Entity Framework v0.3

You are implementing features for lref, an interface-oriented EFCore-inspired
ORM for Rust. Follow the patterns below exactly.

## Quick Reference (common tasks)

| Task | Template |
|------|----------|
| Define an entity | `templates/entity-definition.rs` |
| Create DbContext | `templates/dbcontext.rs` |
| DI container setup | `templates/di-setup.rs` |
| Write queries | `templates/query-patterns.rs` |
| Architecture reference | `references/architecture.md` |

---

## 1. Entities

Use `#[derive(EntityType)]`. Every entity struct needs a `#[table("name")]`
attribute and at least one `#[primary_key]`.

**All available attributes:**

| Attribute | What it does |
|-----------|-------------|
| `#[table("name")]` | Database table name (struct-level) |
| `#[primary_key]` | Primary key column |
| `#[auto_increment]` | Auto-increment / identity |
| `#[required]` | NOT NULL |
| `#[max_length(N)]` | Max string length |
| `#[column("name")]` | Override column name |
| `#[foreign_key(OtherType)]` | Foreign key reference |
| `#[navigation]` | Navigation property (BelongsTo/HasMany/HasOne) |
| `#[not_mapped]` | Exclude from DB mapping |
| `#[index]` | Create index |
| `#[unique]` | Create unique index |
| `#[concurrency_check]` | Optimistic concurrency token |

**Rules:**
- Navigation fields use `BelongsTo<T>`, `HasMany<T>`, `HasOne<T>` ??these are
  pure container types, no trait bounds
- Optional DB columns ??`Option<T>` in Rust
- Read `templates/entity-definition.rs` for the complete pattern

---

## 2. DbContext

The framework provides `DbContext` ??you do NOT define a custom context
struct. No `DbSet<Blog>` fields. Use `ctx.set::<Blog>()` instead.

**Construction:**
```rust
let mut ctx = DbContext::from_options(&options)?;
```

**Entity set access:**
```rust
ctx.set::<Blog>().add(blog);
ctx.set::<Post>().query().to_list().await?;
```
Each `set::<T>()` lazy-creates a `DbSet<T>` and registers a type-erased save
dispatcher. `save_changes()` iterates all entity types automatically.

**Required trait bounds on `set::<T>()`:** `T: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow + Send + Sync + 'static`

Read `templates/dbcontext.rs` for the full setup with migration.

---

## 3. DI Integration (rust-dicore)

Register with `add_dbcontext` (single DB, recommended):
```rust
use rust_dicore::ServiceCollection;
use rust_ef::di::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

let ctx: Arc<dyn IDbContext> = provider.get();
```

**Multiple databases (keyed):**
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

**From pre-built options:**
```rust
let options = DbContextOptionsBuilder::new()
    .connection_string("data source=app.db")
    // ... set_provider_factory(...) etc.
    .build();

.add_dbcontext_from_options::<DbContext>(options)
```

**Provider methods:**
- `use_sqlite(cs)` / `use_sqlite_in_memory()` ??injects factory
- `use_postgres(cs)` ??injects factory
- `use_mysql(cs)` ??tag only (async init)

**How it works:** `use_sqlite()` injects a `provider_factory` closure into
`DbContextOptions`. `DbContext::from_options()` calls this factory to
create the provider. The core crate stays fully decoupled from provider types.

**SaveChanges Interceptors:**
```rust
use rust_ef::interceptor::*;

struct AuditInterceptor;
#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        println!("Saving +{} ~{} -{}", ctx.added_count(), ctx.modified_count(), ctx.deleted_count());
        Ok(())
    }
}

let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|options| {
        options
            .use_sqlite("app.db")
            .add_interceptor(AuditInterceptor);
    })
    .build()
    .unwrap();
```

Read `templates/di-setup.rs` for the complete pattern.

---

## 4. QueryBuilder

`linq!` is the **single DSL entry point** for all database operations. Three
forms are supported:

- **Form A** — filter closure (reusable expression tree or direct query)
- **Form B** — multi-clause query (`;`-separated clauses: `include`,
  `order_by`, `group_by`, `having`, `select`, `inner_join`, `left_join`,
  `sum`/`avg`/`min`/`max`/`count`, `set` + `execute_update`, `take`/`skip`, ...)
- **Form C** — value-producing (for `ModelBuilder`: `filter`, `index`, `key`)

### Form A — filter closure

```rust
use rust_ef::linq;

// Direct ? closest to C# DbSet.Where
linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;

// Reusable expression tree
let expr = linq!(|b: Blog| b.rating > min_rating);
ctx.set::<Blog>().filter(expr).to_list().await?;

// IN clause ? LINQ style: ids.Contains(b.Id)
linq!(ctx.set::<Blog>(), |b: Blog| ids.contains(b.blog_id));
```

### Form B — multi-clause query

```rust
// Eager loading (replaces include_named / then_include_named)
linq!(ctx.set::<Blog>(); include b.posts then b.comments)
    .to_list().await?;

// Ordering + pagination
linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0 => -b.rating)
    .skip(10).take(20).to_list().await?;

// JOIN (replaces inner_join("table", "left_col", "right_col"))
linq!(ctx.set::<Post>(); inner_join |p: Post, b: Blog| p.blog_id == b.blog_id)
    .to_list().await?;

// Grouping + having (replaces group_by(&["col"]) / having("COUNT(*) > 1"))
linq!(ctx.set::<Post>(); group_by b.blog_id; having count(b.post_id) > 1)
    .to_list().await?;

// Aggregates (replaces sum("col") / avg("col"))
let total: f64 = linq!(ctx.set::<Blog>(); sum b.rating).await?;
let avg: f64   = linq!(ctx.set::<Blog>(); avg b.rating).await?;
```

### Form C — ModelBuilder configuration

```rust
// Global query filter (soft delete)
ctx.model().entity::<Blog>()
    .has_query_filter(linq!(filter |b: Blog| b.deleted_at.is_null()));

// Index / key (replaces string-based has_index / has_key)
ctx.model().entity::<Blog>()
    .has_index(linq!(index |b: Blog| (b.author_id, b.created_at)));
ctx.model().entity::<Blog>()
    .has_key(linq!(key |b: Blog| b.blog_id));
```

### Terminals

`to_list()`, `first()`, `first_or_default()`, `last()`, `last_or_default()`,
`single()`, `single_or_default()`, `count()`, `long_count()`, `any()`,
`all(|t| ...)`, `contains(val)`, `to_dictionary(|t| ...)`, plus `linq!` aggregate
terminals (`sum`/`avg`/`min`/`max`/`count`).

### Bulk operations

```rust
// Bulk update (replaces execute_update().set_column("col", val).execute())
let affected = linq!(
    ctx.set::<Blog>(), |b: Blog| b.rating < 3;
    set b.rating, 3;
    execute_update
).await?;

// Bulk delete
let deleted = linq!(ctx.set::<Post>(), |p: Post| p.blog_id == 0)
    .execute_delete().await?;
```

Read `templates/query-patterns.rs` for examples.

---

## 5. Architecture Rules

**Do:**
- All traits are `I`-prefixed (`IDbContext`, `IEntityType`, `IDatabaseProvider`)
- Place trait bounds at usage sites, not on container types
- Use `DbContext` (no custom context struct needed)
- Register via `add_dbcontext::<DbContext>(|o| o.use_sqlite(...))`
- Use `add_dbcontext_keyed::<DbContext>("key", |o| ...)` for multi-DB
- Regster interceptors via `options.add_interceptor(...)`
- Resolve as `Arc<dyn IDbContext>` from DI

**Don't:**
- Define `DbSet<Blog>` struct fields on the context
- Put `IEntityType` bounds on `BelongsTo<T>` or `HasMany<T>`
- Put `IEntityType` bounds on builder structs (`EntityTypeBuilder<T>`)
- Create your own `ServiceCollection` ??use `lrdi::ServiceCollection`

Read `references/architecture.md` for the full architecture documentation.
