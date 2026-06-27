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
    .add_dbcontext::<DbContext>(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

let ctx: Arc<dyn IDbContext> = provider.get();
```

## 多数据库 Keyed 注册

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

## 关键点

| 点 | 说明 |
|---|------|
| `from_options()` 自动发现实体 | 自动调用 `discover_entities()`，无需手动注册元数据 |
| `set::<T>()` 是 lazy 的 | 首次调用时创建 DbSet，重复调用返回同一实例 |
| `ensure_created()` 可直接调用 | 元数据已在 `from_options()` 中自动就绪 |
| `Arc<dyn IDbContext>` 是 object-safe 的 | 支持 DI 容器的 trait object 解析 |

下一节：[第一个 CRUD 流程](first-crud.md)
