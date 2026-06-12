---
name: lref
description: |
  Implement lref (Rust Entity Framework v0.3+) features: define entities with
  #[derive(EntityType)], build DbContext with type-map set storage, register
  via lrdi DI with add_dbcontext<T>(|o| o.use_sqlite("...")), write LINQ-style
  queries, configure migrations, or set up provider extensions. Use when the
  user asks about lref ORM, entity framework, DbContext, or Rust database code.
---

# lref — Rust Entity Framework v0.3

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
- Navigation fields use `BelongsTo<T>`, `HasMany<T>`, `HasOne<T>` — these are
  pure container types, no trait bounds
- Optional DB columns → `Option<T>` in Rust
- Read `templates/entity-definition.rs` for the complete pattern

---

## 2. DbContext

The framework provides `DbContext` — you do NOT define a custom context
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

## 3. DI Integration (lrdi)

Register with `add_dbcontext`:
```rust
use lrdi::ServiceCollection;
use lref::di::*;
use lref_provider_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

let ctx: Arc<dyn IDbContext> = provider.get();
```

**Provider methods:**
- `use_sqlite(cs)` / `use_sqlite_in_memory()` — injects factory
- `use_postgres(cs)` — injects factory
- `use_mysql(cs)` — tag only (async init)

**How it works:** `use_sqlite()` injects a `provider_factory` closure into
`DbContextOptions`. `DbContext::from_options()` calls this factory to
create the provider. The core crate stays fully decoupled from provider types.

Read `templates/di-setup.rs` for the complete pattern.

---

## 4. QueryBuilder

Use `ctx.set::<T>().query()` to get a `QueryBuilder<T>`.

**Filtering:** `filter_column("col", "=", val)`, `filter_in`, `filter_is_null`,
`filter_is_not_null`, `filter_between`

**Ordering/pagination:** `order_by_column`, `order_by_desc_column`, `skip(n)`, `take(n)`

**JOIN:** `inner_join("table", "left_col", "right_col")`, `left_join(...)`

**Grouping:** `group_by(&["col"])`, `having("COUNT(*) > 1")`

**Eager loading:** `include_named("navigation_field")`

**Terminals:** `to_list()`, `first()`, `first_or_default()`, `count()`,
`any()`, `sum("col")`, `avg("col")`

**Bulk:** `execute_update().set_column("col", val).execute()`, `execute_delete()`

Read `templates/query-patterns.rs` for examples.

---

## 5. Architecture Rules

**Do:**
- All traits are `I`-prefixed (`IDbContext`, `IEntityType`, `IDatabaseProvider`)
- Place trait bounds at usage sites, not on container types
- Use `DbContext` (no custom context struct needed)
- Register via `add_dbcontext::<DbContext>(|o| o.use_sqlite(...))`
- Resolve as `Arc<dyn IDbContext>` from DI

**Don't:**
- Define `DbSet<Blog>` struct fields on the context
- Put `IEntityType` bounds on `BelongsTo<T>` or `HasMany<T>`
- Put `IEntityType` bounds on builder structs (`EntityTypeBuilder<T>`)
- Create your own `ServiceCollection` — use `lrdi::ServiceCollection`

Read `references/architecture.md` for the full architecture documentation.
