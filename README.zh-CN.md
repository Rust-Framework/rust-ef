[English](README.md) | **简体中文**

# Rust Entity Framework (rust-ef)

[![Crates.io](https://img.shields.io/crates/v/rust-ef)](https://crates.io/crates/rust-ef)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Rust-Framework/rust-ef/blob/main/LICENSE)
[![Documentation](https://img.shields.io/badge/docs-mdBook-blue.svg)](https://rf2026.github.io/rust-ef/)

一个**受 EFCore 启发的 Rust ORM** —— 提供 `DbContext` / `DbSet<T>` / `IEntityType`，内置 rust-dix 依赖注入（DI）集成、`#[derive(EntityType)]` 实体、`linq!` 宏、变更跟踪（change tracking）、迁移（migrations）以及多数据库支持。

**[在线文档](https://rf2026.github.io/rust-ef/)** —— 使用 mdBook 构建的完整开发者手册。

---

## 快速上手 / Quick Start

```toml
[dependencies]
rust-ef = "1.8"
rust-ef-sqlite = "1.8"
rust-dix = "0.7"
tokio = { version = "1", features = ["full"] }
```

### 定义实体 / Define Entities

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

### 流式配置（自动发现）/ Fluent Configuration (auto-discovered)

`#[derive(EntityType)]` 会在编译期自动注册实体。`DbContext::from_options()` 会自动发现所有已注册实体并应用 `#[entity(T)]` 配置 —— 无需手动调用 `discover_entities()`。

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

### DI 注册与使用（单数据库）/ DI Registration + Usage (Single DB)

```rust
use rust_dix::*;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 注册
    let provider = ServiceCollection::new()
        .add_dbcontext(|options| {
            options.use_sqlite("data source=app.db");
        })
        .build()
        .unwrap();

    // 2. 以 owned DbContext 解析（&mut self 访问，无需锁）
    //    rust-dix 0.6+：get_owned() 返回 Result<T, RdiError>
    let mut ctx: DbContext = provider.get_owned()?;

    ctx.save_changes().await?;
    Ok(())
}
```

### 多数据库（Keyed 注册 + 实体隔离）/ Multi-DB (Keyed Registration + Entity Isolation)

用 `#[context("key")]` 标记实体，使其按 Keyed `DbContext` 隔离。`#[entity(T, "key")]` 只会对匹配的 context 应用配置。

```rust
// 默认 context 实体 —— 无 #[context] 属性
#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog { /* ... */ }

// Keyed context 实体 —— 标记为 "logs" context
#[derive(Debug, Clone, EntityType)]
#[context("logs")]
#[table("log_entries")]
pub struct LogEntry { /* ... */ }

// 注册两个 Keyed DbContext
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
// primary 管理 Blog；logs 管理 LogEntry —— 通过 context_key 隔离
```

### SaveChanges 拦截器 / SaveChanges Interceptors

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

// 注册
.add_dbcontext(|options| {
    options
        .use_sqlite("app.db")
        .add_interceptor(AuditInterceptor);
})
```

---

## 最佳实践指南 / Best Practices Guide

以下模式推荐用于生产环境。它们强调**可读性**与**清晰性**：将复杂操作拆分为有名称的 `let` 绑定，而不是把一切都塞进一条链式调用。

### 推荐的查询风格 / Recommended Query Style

```rust
let set = ctx.set::<Blog>();

let expr = linq!(|b: Blog| b.rating > 0.5);

return set.filter(expr).to_list().await?;
```

这比把所有步骤拼成一条链式调用更清晰，因为每一步都有名称：数据源（`set`）、过滤逻辑（`expr`）和执行（`to_list`）。

### 过滤与排序 / Filtering & Sorting

```rust
let posts = linq!(ctx.set::<Post>(), |p: Post| p.blog_id == target_id && p.title.contains("Rust");
    order_by p.created_at desc;
    skip offset;
    take page_size;
).to_list().await?;
```

### 可复用的 LINQ 表达式 / Reusable LINQ Expressions

```rust
let min_rating = 4;
let high_rated = linq!(|b: Blog| b.rating > min_rating);

let blogs = ctx.set::<Blog>().filter(high_rated).to_list().await?;
let count = ctx.set::<Blog>().filter(high_rated).count().await?;
```

### 导航（预加载）/ Navigation (Eager Loading)

```rust
let blogs = linq!(ctx.set::<Blog>(); include b.posts then b.comments)
    .to_list()
    .await?;
```

### 导航（懒加载，v1.1 起可选）/ Navigation (Lazy Loading, opt-in since v1.1)

```rust
// 在 options 层开启懒加载
let mut options = DbContextOptionsBuilder::new();
options.use_sqlite("app.db").use_lazy_loading(true);
let mut ctx = DbContext::from_options(&options.build())?;

let blogs = ctx.set::<Blog>().query().to_list().await?;
for blog in &blogs {
    // 首次访问时加载导航；后续读取命中缓存
    let posts = blog.posts.load().await?;
    println!("{}: {} posts", blog.url, posts.len());
}
```

> 懒加载是**可选开启**的（`use_lazy_loading(true)`，默认 `false`）。关闭时，请使用 `linq!(...; include ...)` 进行预加载。

### 批量更新 / Bulk Update

```rust
let affected = linq!(
    ctx.set::<Blog>(), |b: Blog| b.rating < 3;
    set b.rating, 3;
    execute_update
).await?;
```

### 批量删除 / Bulk Delete

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

ctx.update::<Blog>(blog);
ctx.save_changes().await?;
```

### 全局查询过滤器（软删除）/ Global Query Filter (Soft Delete)

```rust
ctx.model().entity::<Blog>().has_query_filter(linq!(filter |b: Blog| b.deleted_at.is_null()));
ctx.set::<Blog>();
// 之后的所有查询都会自动追加该过滤表达式
```

### 多数据库（Keyed）/ Multi-DB (Keyed)

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

## Web 应用集成 / Web Application Integration

`DbContext` 通过 `add_dbcontext` 以 **Scoped** 方式注册 —— 每个请求获得自己的实例（工作单元隔离）。无需加锁。

```rust
use std::sync::Arc;
use rust_ef::db_context::DbContext;
use rust_ef::di::*;

// 以 Scoped 注册（类似 ASP.NET Core 的 AddDbContext<T>）
let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

// 每个请求都会创建一个 scope。处理器通过 get_owned() 拥有一个全新的 DbContext。
// rust-dix 0.6+：get_owned() 返回 Result<T, RdiError>
let mut ctx: DbContext = provider.get_owned().unwrap();

// 通过 DI 注入到处理器 —— 标注了 #[inject(owned)] 的裸 T 字段 → owned 解析
#[derive(Inject)]
pub struct MyHandler {
    #[inject(owned)]
    ctx: DbContext,
}

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<MyRequest, MyResponse> for MyHandler {
    async fn handle(&mut self, req: MyRequest) -> Result<MyResponse> {
        self.ctx.add::<Blog>(blog);
        self.ctx.save_changes().await?;
        // ...
    }
}
```

> **`Arc<Mutex<DbContext>>` 是一种反模式**：它会造成跨请求的跟踪污染 —— 线程 A 的
> `save_changes()` 会提交线程 B 的待保存更改。
> 请改用 owned 解析（`get_owned()`）或 Scoped 生命周期，这与 EFCore 的设计保持一致。

### 推荐模式 / Recommended patterns

```rust
// ✓ 逐步 let 绑定 —— 可读且易于调试
let set = ctx.set::<Blog>();
let expr = linq!(|b: Blog| b.slug == req.slug);
let exists = set.filter(expr).first_or_default().await?;

// ✓ linq! 表达式绑定 —— 过滤逻辑独立命名
let expr = linq!(|b: Blog| b.rating > 3);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;

// ✓ 创建流程：构建 → 插入 → 保存 → 按主键重新查询（用于导航）
let mut blog = req.to_entity(uid, now);
ctx.add::<Blog>(blog);
ctx.save_changes().await?;
// blog.id 现在已填充 —— 无需仅仅为了拿到 ID 而重新查询

// 仅当你需要导航属性时才重新查询，且始终按主键查询
let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
    include b.category;
).first_or_default().await?;
```

---

## 常见陷阱与反模式 / Common Pitfalls & Anti-Patterns

### 不要仅仅为了自增 ID 而重新查询

```rust
// ❌ 错误：id 已在实体上
ctx.add::<Blog>(blog);
ctx.save_changes().await?;
let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q).first_or_default().await?;
let id = saved.unwrap().id;

// ✅ 正确：直接使用实体
ctx.add::<Blog>(blog);
ctx.save_changes().await?;
let id = blog.id; // 已被填充！
```

### 不要使用基于字符串的列名

```rust
// ❌ 错误：没有编译期检查
ctx.set::<Blog>().query().filter_column("slug", "=", "hello").to_list().await?;

// ✅ 正确：类型安全的 linq! 表达式
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == "hello").to_list().await?;
```

### 不要在每个查询里重复 `is_deleted` —— 使用全局查询过滤器

```rust
// ❌ 错误：重复、容易遗漏
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q && !b.is_deleted)

// ✅ 正确：启动时注册一次
ctx.model().entity::<Blog>()
    .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));
// 所有查询现在都会自动排除已删除的记录

// 需要看到全部记录的管理员查询：
ctx.set::<Blog>().query_ignore_filters().to_list().await?;
```

### 不要使用 `Arc<Mutex<DbContext>>` —— 使用 owned 解析

```rust
// ❌ 错误：跨请求的跟踪污染
#[derive(Inject)]
pub struct MyHandler {
    ctx: Arc<Mutex<DbContext>>,
}

// ✅ 正确：Scoped 注册 + owned 解析，每个请求获得自己的实例
// main.rs:
.add_dbcontext(|o| o.use_sqlite("app.db"));
// handler:
#[derive(Inject)]
pub struct MyHandler {
    #[inject(owned)]
    ctx: DbContext,  // 裸 T + #[inject(owned)] → get_owned()
}

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<MyRequest, MyResponse> for MyHandler {
    async fn handle(&mut self, req: MyRequest) -> Result<MyResponse> {
        self.ctx.add::<Blog>(blog);
        self.ctx.save_changes().await?;
        // ...
    }
}
```

### 修改时优先使用 `detect_changes()` 而不是 `update()`

```rust
// ✓ 不够精准：update() 会把整个实体标记为 Modified
ctx.update::<Blog>(blog);
ctx.save_changes().await?;

// ✓ 更好：detect_changes() 只标记实际发生变更的字段
blog.is_deleted = true;
ctx.detect_changes();
ctx.save_changes().await?;
```

---

## 完整文档 / Full Documentation

完整的实践手册请参见 [`docs/rust-ef/INDEX.md`](https://github.com/Rust-Framework/rust-ef/blob/main/docs/rust-ef/INDEX.md)，涵盖：

- 使用 `#[derive(EntityType)]` 进行实体设计
- 一对多与多对多关系
- 进阶查询：聚合、GROUP BY、JOIN、原生 SQL
- 变更跟踪：Add / Attach / Update / Remove
- 批量操作、事务与迁移
- DI 集成、拦截器与多数据库模式
- 常见陷阱、性能技巧与代码评审清单

---

## 架构 / Architecture

```
User Application
    └── rust-dix (crates.io, DI, resolves Arc<DbContext>)
    └── rust-ef (ORM, workspace: crates/core)
          DbContext (type-map set storage, no entity-specific fields)
          ├── DbContext       (concrete session/unit-of-work type)
          ├── IDbSet<T>       (entity collection / mutation)
          ├── IQueryable<T>   (query entry point)
          ├── ISaveChangesInterceptor (before/after save hooks)
          └── IDatabaseProvider (backend abstraction)
                ├── crates/sqlite    (rust-ef-sqlite    via use_sqlite)
                ├── crates/postgres  (rust-ef-postgres  via use_postgres)
                └── crates/mysql     (rust-ef-mysql     via use_mysql)
```

### 接口层级 / Interface Hierarchy

```
IEntityType ─ IFromRow
            ├ IGetKeyValues
            └ IEntitySnapshot

IQueryable<T> ─ IDbSet<T>

DbContext (concrete context type)
    ├ provider()             → &dyn IDatabaseProvider
    ├ save_changes()         → SaveChangesResult
    └ change_tracker()       → &ChangeTracker

IDatabaseProvider
    ├ sql_generator()        → ISqlGenerator
    ├ get_connection()       → IAsyncConnection
    └ execute_migration_command(sql)

ISaveChangesInterceptor
    ├ on_saving(ctx)           // pre-commit; Err aborts save
    ├ on_saved(ctx, result)    // post-commit
    └ on_save_failed(ctx, err) // on error (after rollback)

FromDbContextOptions (DI bridge)
    └ from_options(&DbContextOptions) → Self
```

---

## 关键设计决策 / Key Design Decisions

| 决策 | 理由 |
|----------|-----------|
| 没有 `DbSet<Blog>` 结构体字段 | `DbContext` 使用 type-map；set 通过 `set::<T>()` 惰性创建 |
| options 中的 `provider_factory` | 各 provider 扩展注入工厂闭包；core 保持解耦 |
| `SetOps<T>` 分发器 | 类型擦除的 `save_changes()` 可遍历所有实体类型 |
| 多数据库的 Keyed 注册 | `add_dbcontext_keyed` + `provider.get_keyed()` |
| 拦截器管道 | `options.add_interceptor(...)` 用于横切关注点 |

---

## 特性 / Features

| 分类 | 特性 |
|----------|---------|
| **实体 Entity** | `#[derive(EntityType)]` 支持 14 种属性、导航类型、自动发现 |
| **查询 Query** | `linq!` 表达式树、`filter` / `filter_column`、join、group_by、聚合、IN/NOT IN 子查询 |
| **进阶查询 Advanced Query** | CTE（`linq!(with ...)` 语法糖）、窗口函数（10 种）、懒加载（可选） |
| **持久化 Persistence** | `save_changes()`、参数化查询、事务、级联删除（v1.5.2） |
| **DI 依赖注入** | `add_dbcontext` / `add_dbcontext_keyed` / `add_dbcontext_from_options`、`Arc<DbContext>`、多数据库 context key 隔离 |
| **拦截 Interception** | `ISaveChangesInterceptor` —— on_saving/on_saved/on_save_failed 钩子 |
| **迁移 Migrations** | 模型差异、Up/Down SQL、历史跟踪、`MigrationStore`、外键 ON DELETE 子句（v1.5.2） |
| **CLI** | `rust-ef-cli`：`migration init/add/apply/revert/list/script`、`scaffold dbcontext` |

---

## 派生属性 / Derive Attributes

| 属性 | EFCore 对应项 |
|-----------|-------------------|
| `#[table]` | `[Table]` |
| `#[primary_key]` | `[Key]` |
| `#[auto_increment]` | 约定 |
| `#[required]` | `[Required]` |
| `#[max_length]` | `[MaxLength]` |
| `#[column]` | `[Column]` |
| `#[foreign_key]` | `[ForeignKey]` |
| `#[navigation]` | 隐式 |
| `#[not_mapped]` | `[NotMapped]` |
| `#[index]` / `#[unique]` | `[Index]` |
| `#[concurrency_check]` | `[ConcurrencyCheck]` |
| `#[context("key")]` | 多数据库 context key 隔离（v1.1） |
| `#[on_delete]` | `OnDelete(DeleteBehavior)` Fluent API（v1.5.2） |

---

## 许可证 / License

MIT