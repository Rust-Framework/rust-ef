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
use rust_dix::*;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

// 推荐：owned 解析，直接 &mut self 访问（调用方持有，不进容器缓存）
// rust-dix 0.10: get_owned() 返回 Result<T, RdiError>
let mut ctx: DbContext = provider.get_owned().unwrap();

// 或：共享解析（Arc<DbContext>，&self 访问，同一 scope 内共享）
// rust-dix 0.10: Scoped 服务从根解析会被严格 scope 校验拒绝，需先建 scope
// let scope = provider.scope();
// let ctx: Arc<DbContext> = scope.get().unwrap();
```

> `DbContext` 是 Scoped 生命周期。`rust-dix 0.10` 默认严格 scope 校验：根 `provider.get::<DbContext>()`
> 返回 `ScopedFromRootProvider`；`get_owned()` / `get_keyed_owned()` 仍可从根解析（调用方持有）。

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

// Owned 解析（推荐，可从根解析）：
let mut primary: DbContext = provider.get_keyed_owned("primary").unwrap();
let mut logs: DbContext = provider.get_keyed_owned("logs").unwrap();

// 共享解析（必须在 scope 内）：
// let scope = provider.scope();
// let primary: Arc<DbContext> = scope.get_keyed("primary").unwrap();
```

## 从 rust-dix 0.7 迁移

0.7 的根 provider 会把 Scoped 服务缓存为根级单例。迁移期如仍需旧行为，可用
`build_with(ServiceProviderOptions::lenient())`；长期请改用 `provider.scope()` + `get_owned()`。
详见 [rust-dix 集成与注册模式](../10-di-interceptors/di-registration.md)。

## 关键点

| 点 | 说明 |
|---|------|
| `from_options()` 自动发现实体 | 自动调用 `discover_entities()`，无需手动注册元数据 |
| `set::<T>()` 是 lazy 的 | 首次调用时创建 DbSet，重复调用返回同一实例 |
| `ensure_created()` 可直接调用 | 元数据已在 `from_options()` 中自动就绪 |
| Owned 解析（推荐） | `get_owned()` → `DbContext`，`&mut self` 访问，无需锁 |
| Shared 解析 | `scope.get()` → `Arc<DbContext>`，`&self` 访问，同一 scope 内共享 |

下一节：[第一个 CRUD 流程](first-crud.md)
