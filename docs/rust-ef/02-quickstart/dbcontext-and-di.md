# DbContext 与 DI 注册

`DbContext` 是工作单元的入口。与传统 ORM 不同，`rust-ef` 的 `DbContext` 使用**类型映射（type-map）**存储 `DbSet`，无需为每个实体定义字段。

## 手动创建

```rust
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let mut builder = DbContextOptionsBuilder::new();
builder.use_sqlite("app.db");
let mut ctx = DbContext::from_options(&builder.build())?;
// from_options() 自动发现所有 #[derive(EntityType)] 标注的实体
// 并应用所有 #[entity(T)] 配置 —— 无需手动调用 discover_entities()

ctx.ensure_created().await?;  // 直接建表，元数据已就绪
```

## DI 注册（推荐）

```rust
use rust_dicore::ServiceCollection;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

// 推荐：owned 解析，直接 &mut self 访问
let mut ctx: DbContext = provider.get_owned();

// 或：共享解析（Arc<DbContext>，&self 访问，同一 scope 内共享）
// let scope = provider.create_scope();
// let ctx: Arc<DbContext> = scope.get();
```

## 多数据库 Keyed 注册

```rust
let provider = ServiceCollection::new()
    .add_dbcontext_keyed("primary", |options| {
        options.use_postgres("host=primary/db");
    })
    .add_dbcontext_keyed("logs", |options| {
        options.use_sqlite("logs.db");
    })
    .build()
    .unwrap();

// Owned 解析（推荐）：
let mut primary: DbContext = provider.get_keyed_owned("primary");
let mut logs: DbContext = provider.get_keyed_owned("logs");

// 共享解析：
// let primary: Arc<DbContext> = scope.get_keyed("primary");
```

## 关键点

| 点 | 说明 |
|---|------|
| `from_options()` 自动发现实体 | 自动调用 `discover_entities()`，无需手动注册元数据 |
| `set::<T>()` 是 lazy 的 | 首次调用时创建 DbSet，重复调用返回同一实例 |
| `ensure_created()` 可直接调用 | 元数据已在 `from_options()` 中自动就绪 |
| Owned 解析（推荐） | `get_owned()` → `DbContext`，`&mut self` 访问，无需锁 |
| Shared 解析 | `scope.get()` → `Arc<DbContext>`，`&self` 访问，同一 scope 内共享 |

下一节：[第一个 CRUD 流程](first-crud.md)
