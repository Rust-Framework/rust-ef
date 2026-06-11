# Rust Entity Framework (lref)

[![Crates.io](https://img.shields.io/crates/v/lref)](https://crates.io/crates/lref)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

An EFCore-inspired ORM for Rust, bringing the familiar `DbContext` / `DbSet` / `EntityType` patterns to the Rust ecosystem.

---

## Quick Start

Add to `Cargo.toml`:

```toml
[dependencies]
lref = "0.1"
lref-provider-sqlite = "0.1"  # or postgres / mysql
tokio = { version = "1", features = ["full"] }
```

Define entities with `#[derive(EntityType)]`:

```rust
use lref::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,

    #[required]
    #[max_length(200)]
    pub url: String,

    pub rating: i32,

    #[navigation]
    pub posts: HasMany<Post>,
}

#[derive(Debug, Clone, EntityType)]
#[table("posts")]
pub struct Post {
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    pub content: Option<String>,

    #[foreign_key(Blog)]
    pub blog_id: i32,

    #[navigation]
    pub blog: BelongsTo<Blog>,
}
```

Define `DbContext` and use `save_changes_all!` for auto-persistence:

```rust
use lref::prelude::*;
use lref::save_changes_all;
use lref_provider_sqlite::SqliteProvider;
use std::sync::Arc;

pub struct BloggingContext {
    pub blogs: DbSet<Blog>,
    pub posts: DbSet<Post>,
    change_tracker: ChangeTracker,
    provider: Arc<SqliteProvider>,
}

#[async_trait::async_trait]
impl DbContext for BloggingContext {
    type Provider = SqliteProvider;
    fn provider(&self) -> &Self::Provider { &self.provider }
    fn change_tracker_mut(&mut self) -> &mut ChangeTracker { &mut self.change_tracker }
    fn change_tracker(&self) -> &ChangeTracker { &self.change_tracker }
    async fn save_changes(&mut self) -> Result<SaveChangesResult, LrefError> {
        save_changes_all!(self, blogs, posts)
    }
}
```

Full CRUD:

```rust
#[tokio::main]
async fn main() -> Result<(), LrefError> {
    let provider = Arc::new(SqliteProvider::new_in_memory()?);
    let mut ctx = BloggingContext {
        blogs: DbSet::with_provider("blogs", provider.clone()),
        posts: DbSet::with_provider("posts", provider.clone()),
        change_tracker: ChangeTracker::new(),
        provider,
    };

    // Create table (via migration engine)
    let engine = lref::migration::MigrationEngine::new(lref::migration::MigrationDialect::Sqlite);
    let migration = engine.generate("init", &[Blog::entity_meta(), Post::entity_meta()], &None)?;
    ctx.provider().execute_migration_command(&migration.up_sql).await?;

    // Add & save
    ctx.blogs.add(Blog { blog_id: 0, url: "https://example.com".into(), rating: 5, posts: HasMany::new() });
    ctx.save_changes().await?;

    // Query
    let blogs = ctx.blogs.query()
        .filter_column("rating", ">", 3)
        .order_by_column("url")
        .to_list().await?;
    println!("Found {} blogs", blogs.len());

    // Count / aggregate
    let count = ctx.posts.query().count().await?;
    let avg_rating = ctx.blogs.query().avg("rating").await?;

    Ok(())
}
```

---

## Features

| Category | Feature | Status |
|----------|---------|--------|
| **Entity Modeling** | `#[derive(EntityType)]` with 12 field attributes | ✅ |
| | Primary key, auto-increment, required, max_length | ✅ |
| | Column name override, foreign key, index, unique | ✅ |
| | Navigation properties: `BelongsTo<T>`, `HasMany<T>`, `HasOne<T>` | ✅ |
| | Composite primary key | ✅ |
| | Optimistic concurrency (`#[concurrency_check]`) | ✅ |
| **Query** | LINQ-style fluent `QueryBuilder<T>` | ✅ |
| | `filter_column`, `filter_in`, `filter_is_null`, `filter_between` | ✅ |
| | `order_by`, `skip`, `take`, `include` | ✅ |
| | `inner_join`, `left_join`, `group_by`, `having` | ✅ |
| | Aggregation: `sum`, `avg`, `min`, `max`, `count` | ✅ |
| | `execute_update`, `execute_delete` (bulk) | ✅ |
| | Global query filters (`ModelBuilder::has_query_filter`) | ✅ |
| **Persistence** | Generic `save_changes_all!` — auto INSERT/UPDATE/DELETE | ✅ |
| | Parameterized queries (no SQL injection) | ✅ |
| | Transaction support | ✅ |
| | Change tracker with property-level snapshots | ✅ |
| **Migrations** | Model diff (add/drop tables & columns, alter, FK) | ✅ |
| | Up/Down SQL generation for PostgreSQL, MySQL, SQLite | ✅ |
| | `__ef_migrations_history` tracking table | ✅ |
| **Database Providers** | PostgreSQL (`deadpool-postgres`) | ✅ |
| | MySQL (`sqlx`) | ✅ |
| | SQLite (`rusqlite`) | ✅ |
| **Tooling** | CLI: `lref migration add/apply/revert/list/script` | ✅ |
| | CLI: `lref scaffold-dbcontext` (reverse engineer DB) | ✅ |
| | `column!()` macro for compile-time column name resolution | ✅ |

---

## Architecture

```
examples/blog/          User application
    ↓
crates/lref/            Core ORM (EntityType, DbContext, QueryBuilder, MigrationEngine)
    ↓
crates/lref-macros/     #[derive(EntityType)]  +  column!()  proc macros
    ↓
crates/lref-provider-*  Database drivers (PostgreSQL / MySQL / SQLite)
    ↓
crates/lref-cli/        CLI tool (migrations + scaffold)
```

---

## Database Providers

| Crate | Database | Connection Pool |
|-------|----------|----------------|
| [`lref-provider-postgres`](https://crates.io/crates/lref-provider-postgres) | PostgreSQL | `deadpool-postgres` |
| [`lref-provider-mysql`](https://crates.io/crates/lref-provider-mysql) | MySQL | `sqlx` |
| [`lref-provider-sqlite`](https://crates.io/crates/lref-provider-sqlite) | SQLite | `Arc<Mutex<Connection>>` |

---

## Derive Macro Attributes

| Attribute | EFCore Equivalent | Description |
|-----------|-------------------|-------------|
| `#[table("name")]` | `[Table("name")]` | Database table name (struct-level) |
| `#[primary_key]` | `[Key]` | Primary key column |
| `#[auto_increment]` | (convention) | Auto-increment / identity |
| `#[required]` | `[Required]` | NOT NULL constraint |
| `#[max_length(N)]` | `[MaxLength(N)]` | Max string length |
| `#[column("name")]` | `[Column("name")]` | Different column name |
| `#[foreign_key(T)]` | `[ForeignKey]` | Foreign key reference |
| `#[navigation]` | (implicit) | Navigation property |
| `#[not_mapped]` | `[NotMapped]` | Exclude from mapping |
| `#[index]` | `[Index]` | Create database index |
| `#[unique]` | (unique index) | Create unique index |
| `#[concurrency_check]` | `[ConcurrencyCheck]` | Optimistic concurrency token |

---

## Development Status

Current: **Alpha 2 → Beta 1** (core CRUD + query capabilities complete, see [spec](docs/PRODUCTION_READINESS_SPEC.md))

Upcoming:
- Eager-loading navigation property materialization
- `OR` condition support in query expressions
- Subquery / correlated subquery support
- CLI migration `apply` with live database connection
- User guide (mdBook)

---

## License

MIT — see [LICENSE](LICENSE)
