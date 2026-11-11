# rust-dix 集成与注册模式

`rust-ef` 与 `rust-dix` DI 容器深度集成，支持构造函数注入和 owned/shared 两种解析模式。
`rust-ef` 只提供 `add_dbcontext*` 这一个集成面（`crates/core/src/di.rs`），
生命周期、作用域缓存、owned 解析、释放顺序与 MEDI 注册语义全部由 `rust-dix` 实现，EF 不重复实现容器。

> 版本：`rust-dix 0.10`（类型化 ServiceKey、AnyKey、AsyncFactoryContext、严格 scope、确定性生命周期）。
> `rust-ef 2.1` 要求 `rust-dix ^0.10` 与 **Rust 1.95+**。

## 注册变体（2.1）

| API | provider 初始化时机 | 适用场景 |
|-----|---------------------|----------|
| `add_dbcontext` | 注册时（失败 panic，快速失败） | 最简单启动路径 |
| `add_dbcontext_keyed` | 注册时 | 静态读/写、模块上下文 |
| `add_dbcontext_anykey` | 首次解析某 key 时（按 key 缓存） | 动态租户/分片 |
| `add_dbcontext_async` / `add_dbcontext_keyed_async` | `build_async().await` 中（失败返回 `RdiError`） | 异步启动、需要类型化启动错误 |

## 类型化 context key（含 Handler 注入）

`#[derive(DbContextKey)]` 生成 `IDbContextKey`（实体模型字符串名）与 rust-dix 0.10 官方扩展
契约 `From<Key> for ServiceKey`（typed key），注册、解析和 Handler 注入共享同一 key：

```rust
use rust_ef::db_context::DbContext;
use rust_ef::di::DbContextResolver;
use rust_ef::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, DbContextKey)]
enum Db { Read, Write }

let provider = ServiceCollection::new()
    .add_dbcontext_keyed(Db::Read, |o| { o.use_default_entity_model(); o.use_sqlite("read.db"); })
    .add_dbcontext_keyed(Db::Write, |o| { o.use_default_entity_model(); o.use_sqlite("write.db"); })
    .build()
    .unwrap();

let mut write = provider.get_owned_context(Db::Write).unwrap(); // 根上可用（owned）
let scope = provider.scope();
let read = scope.get_context(Db::Read).unwrap();               // 共享解析需 scope

// rust-dix 0.10 宏 key 表达式：
#[inject(scoped)]
struct WriteHandler {
    #[inject(owned, key = Db::Write)]
    ctx: DbContext,
}
```

- enum 需派生 `Debug + Clone + Copy + PartialEq + Eq + Hash`；键名默认 variant 的 snake_case，
  可用 `#[key("custom")]` 覆盖；实体 `#[context("...")]` 必须与 `name()` 一致。
- 手工实现 `IDbContextKey` 时必须同时实现 `From<Key> for ServiceKey`（官方扩展契约），
  或使用派生宏；不要复制 rust-dix 注入宏。
- 字符串 key（`"read"`）仍注册为 `String` key，与 typed enum key 不互通。

## AnyKey 动态上下文（租户/分片）

```rust
let provider = ServiceCollection::new()
    .add_dbcontext_anykey(|key, options| {
        let tenant = key.to_string(); // 必须经白名单/租户表映射
        options.use_postgres(tenant_connection_string(&tenant));
    })
    .build()
    .unwrap();

let scope = provider.scope();
let mut ctx = scope.get_keyed_owned::<DbContext>("tenant-a").unwrap();
```

- 精确 key 注册优先；options/provider 按请求 key 缓存，跨 scope 复用连接池。
- 缓存为**有界 LRU**（默认 256 个 key，`add_dbcontext_anykey_with_capacity` 可调）；
  淘汰只移除缓存中的 options，活跃 `DbContext` 仍持有 provider `Arc`，之后同 key 会重建连接池。
- AnyKey 上下文强制使用默认实体模型；租户隔离由查询过滤器负责。
- 安全：禁止把请求 key 直接拼入 SQL/连接串；key 基数需受控。
- 异步 AnyKey 尚未提供。

## 异步 provider 初始化

```rust
let provider = ServiceCollection::new()
    .add_dbcontext_async(|options| {
        options.use_postgres("postgres://app/db");
    })
    .build_async()
    .await
    .unwrap();

// DbContext 仍是同步 Scoped 服务，owned 注入不变
let mut ctx: DbContext = provider.get_owned().unwrap();
```

- provider 创建在 `build_async` 中执行，失败返回类型化 `RdiError`（不再注册期 panic）。
- 可选异步自检：`options.async_preflight(|options| Box::pin(async move { ... }))` 在
  provider/连接池创建后、`build_async` 完成前执行；返回 `Err` 会中止启动。
- 同步 `build()` 遇到异步注册返回 `AsyncServiceRequiresBuildAsync`。
- keyed 变体：`add_dbcontext_keyed_async(key, configure)`。

## rust-dix 0.10 能力说明

- **类型化 `ServiceKey` + 宏 key 表达式**：EF 用 `#[derive(DbContextKey)]` 同时生成
  `IDbContextKey` 与官方扩展契约 `From<Key> for ServiceKey`，因此注册、`DbContextResolver`
  和 `#[inject(owned, key = Db::Read)]` 使用同一个 typed key。
- **AnyKey** 通过 `add_dbcontext_anykey` 暴露（有界 LRU、精确 key 优先、默认实体模型）。
- keyed API 接受 `impl Into<ServiceKey>`；`&str` / `String` 源码兼容。注意
  `ServiceKey::new("x")` 是 `&str` 类型 key，不等同于字符串注册的 `String` key。
- key 表达式在字段构造时求值一次（singleton 在 build/validation 一次、transient 每次解析
  一次），应保持稳定、无副作用。
- async singleton 依赖 Scoped `DbContext` 会得到
  `SyncSingletonDependsOnAsyncService`；异步服务被同步解析会得到
  `AsyncServiceRequiresGetAsync`——应把依赖方改为 async 或延迟解析。
- `async_any_*` 仍未提供；AnyKey 保持同步初始化。
- 完整的 0.10 能力评估、实现边界与后续路线见仓库 `docs/rust-dix-0.10-adoption.md`。

## 基础注册

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
let mut ctx: DbContext = provider.get_owned().unwrap();
```

## Scoped 生命周期与严格 scope 校验（0.8 默认）

`DbContext` 注册为 **Scoped**。`rust-dix 0.10` 默认启用 MEDI `ValidateScopes` 语义：
**从根 `ServiceProvider` 解析 Scoped 服务返回 `RdiError::ScopedFromRootProvider`**（根 provider 不是 scope）。

按请求 / 后台作业创建 scope，在 scope 内解析：

```rust
let scope = provider.scope();          // rust-dix 0.10 的推荐写法
let mut ctx: DbContext = scope.get_owned().unwrap();  // owned，&mut self
// 或共享解析：
// let ctx: Arc<DbContext> = scope.get().unwrap();    // 同一 scope 内共享
```

例外：`get_owned` / `get_keyed_owned`（以及 `#[inject(owned)]`）**允许从根解析 Scoped**——
实例归调用方所有，容器不缓存、不跟踪；但它构造函数内解析的 Scoped 依赖仍要遵守 scope 校验。
`rust-webx` 的 HTTP 分发会为每个请求自动创建 scope，Handler 通过 `#[inject(owned)]` 使用请求级
`DbContext`，无需手写 scope 管理。

### 从 rust-ef ≤ 2.0（rust-dix 0.7）迁移

0.7 的根 provider 会把 Scoped 服务当成根级单例缓存。若暂时无法把所有根解析改为 scope，可用
`build_with(ServiceProviderOptions::lenient())` 恢复旧行为：

```rust
use rust_ef::di::{ServiceCollection, ServiceProviderOptions};

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build_with(ServiceProviderOptions::lenient())  // validate_scopes = false
    .unwrap();
```

> `lenient` 只是迁移逃生门。长期应改为 `provider.scope()` + `get_owned()`，避免“根解析 Scoped
> 变成应用级单例”的隐性 bug。

## 在 Handler 中注入

`#[derive(Inject)]` 要求 bare `T` 字段标记 `#[inject(owned)]`（`Arc<T>` 字段标记 `#[inject]`），
未标记字段走 `Default::default()`。Handler 方法使用 `&mut self`：

```rust
use webx::*;   // rust-webx（Web 框架）
use rust_ef::db_context::DbContext;

#[derive(Inject)]
pub struct ListBlogsHandler {
    #[inject(owned)]
    ctx: DbContext,  // bare T + #[inject(owned)] → get_owned()
}

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<ListBlogsRequest, Vec<BlogDto>> for ListBlogsHandler {
    async fn handle(&mut self, _req: ListBlogsRequest) -> Result<Vec<BlogDto>> {
        let blogs = self.ctx.set::<Blog>().query().to_list().await?;
        Ok(blogs.into_iter().map(BlogDto::from).collect())
    }
}
```

若 Handler 同时注入共享的 `Arc<DbContext>`，该 Handler 必须从 scope 解析（否则构造函数里的
`get()` 会触发严格校验）：

```rust
let scope = provider.scope();
let handler: MyHandler = scope.get_owned().unwrap();
```

## Repository 封装模式

```rust
pub struct BlogRepository {
    ctx: DbContext,
}

impl BlogRepository {
    pub async fn list_high_rated(&mut self) -> EFResult<Vec<Blog>> {
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.rating > 4);
        set.filter(expr).to_list().await
    }
}
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

// Owned keyed 解析（推荐；可从根解析）
let mut primary: DbContext = provider.get_keyed_owned("primary").unwrap();
let mut logs: DbContext = provider.get_keyed_owned("logs").unwrap();

// 共享 keyed 解析必须在 scope 内
let scope = provider.scope();
let primary: Arc<DbContext> = scope.get_keyed("primary").unwrap();
```

## 注册语义与释放

| 语义 | rust-dix 0.10 行为 | 对 rust-ef 的影响 |
|------|------------------|-------------------|
| 单服务解析 | **后注册优先**（MEDI） | 同一个 key 重复 `add_dbcontext` / `add_dbcontext_keyed` 时，最后一次生效 |
| `try_add_*` | 仅当未注册时添加 | 需要“默认实现可被覆盖”时使用 |
| `get_all` | 只返回非 keyed 注册，按注册顺序 | EF 不依赖“多实现收集”；拦截器是 `DbContextOptions` 上的显式列表 |
| 严格 scope 校验 | 默认开启，`lenient()` 可关 | 根上 `get::<DbContext>()` / `get_keyed()` 报 `ScopedFromRootProvider` |
| 释放契约 | scope 跟踪 Scoped 实例，`dispose()`/`Drop` 逆序释放；`provider.dispose()` 释放单例 | 请求结束 `drop(scope)` 即可释放 `DbContext`，EF 无需自建清理表 |
| 释放后解析 | `ScopeDisposed` / `ProviderDisposed` | 停机后不要继续 dispatch |

> 注意：EF 的连接池保存在 `DbContextOptions`（跨 scope 共享），scope 释放的是 `DbContext` 本身；
> 应用停机时调用 `provider.dispose()` 释放单例资源。

## 职责边界（rust-ef vs rust-dix）

`rust-ef` 不重复实现 DI 能力，避免与 `rust-dix` 出现两套生命周期/释放语义：

| 能力 | 归属 | rust-ef 的做法 |
|------|------|----------------|
| 注册、构建、作用域、owned 解析 | rust-dix | 只在 `di.rs` 调用 `scoped` / `keyed_scoped` |
| 严格 scope 校验（根 provider 拒绝 Scoped） | rust-dix 0.10 | 不自建校验，直接返回 `RdiError::ScopedFromRootProvider` |
| Scoped 实例跟踪与释放顺序 | rust-dix 0.10 | 不自建清理表，`drop(scope)` / `provider.dispose()` 即可 |
| MEDI 注册顺序（last-wins / `try_add_*`） | rust-dix 0.10 | 原样透传，不在 EF 侧重排 |
| 连接池 | rust-ef（`DbContextOptions`） | 进程级共享，跨 scope 复用，避免每请求建池 |
| `DbContext` 的工作单元与事务 | rust-ef | 与 DI 无关；事务生命周期由 `DbContext` 自己管理 |

## 设计要点

| 实践 | 说明 |
|------|------|
| Owned 解析（推荐） | `get_owned()` → `DbContext`，`&mut self` 访问，无需锁 |
| Shared 解析 | `scope.get()` → `Arc<DbContext>`，`&self` 访问，同一 scope 内共享 |
| 每个请求一个 DbContext | Scoped 生命周期，避免跨请求跟踪污染 |
| 不手写 scope 缓存/释放 | 交给 rust-dix 0.10 的 scope 跟踪与 `dispose()` |
| `#[derive(Inject)]` 显式标记 | bare `T` + `#[inject(owned)]` → owned；`Arc<T>` + `#[inject]` → shared；未标记 → `Default` |

下一节：[多数据库 Keyed 注册与读写分离](keyed-databases.md)
