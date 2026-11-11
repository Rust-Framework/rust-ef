# rust-dix 0.10 能力评估与 rust-ef 2.1 采纳矩阵

> 依据：rust-dix 0.10.0（2026-11-11）CHANGELOG 与源码（`D:\GitCode\RF\rust-dix` v0.10.0）。
> 结论对应 rust-ef 2.1.0 的实现状态。

## 1. 结论摘要

- **0.10 关闭了上一版最大的边界**：`#[inject(key = <expr>)]` / `#[inject(owned, key = <expr>)]`
  接受任意 `impl Into<ServiceKey>`，官方扩展路径是下游实现 `From<LocalKey> for ServiceKey`。
  rust-ef 据此新增 `#[derive(DbContextKey)]`，让类型化 key 同时覆盖**注册、解析与 Handler 注入**。
- **0.10 对 0.9 源码兼容**：字符串 key 语义、`.await` 与现有 EF 代码不变；MSRV 仍为 **1.95**。
- **已采纳**：类型化 key（含派生宏注入）、AnyKey 动态上下文（有界 LRU）、异步 provider 初始化
  （含 async preflight）、rust-dix 的类型化 resolver 扩展、MSRV/安全/CI 基线。
- **仍保留的边界**：异步 AnyKey（dix 0.10 明确未提供）、`#[context]` 枚举化（3.0 设计）、
  `rust-webx` 需升级到 dix 0.10 后才能组合发布。

## 2. 能力 → 2.1 决策

| 0.10 能力 | 2.1 决策 | 实现/边界 |
|-----------|----------|-----------|
| **`ServiceKey` 类型化 key** | ✅ 采用 | `IDbContextKey` + `DbContextResolver`；枚举 key 注册为 typed `ServiceKey`，字符串 key 仍为 `String` key |
| **宏 key 表达式（`#[inject(key = <expr>)]`）** | ✅ 采用 | 新增 `#[derive(DbContextKey)]` 生成 `IDbContextKey` 与 `From<Key> for ServiceKey`；Handler 可写 `#[inject(owned, key = DbKey::Write)]` |
| **扩展契约（`From<LocalKey> for ServiceKey`，勿复制注入宏）** | ✅ 采用 | 派生宏按官方契约生成 `From`；不复制 rust-dix 注入宏 |
| **typed-keyed resolver 扩展（`*_key` 方法）** | ✅ 采用 | async keyed options 解析使用 `get_keyed_any_key`；其余路径使用 typed keyed API |
| **`register!(keyed ... => Impl)` 修复** | 不适用 | EF 通过 Rust 注册 API（`add_dbcontext*`），不使用 `rust_dix::module` 宏 |
| **`rdi-E004` 编译期重复 key 检测** | 不适用 | EF 注册在运行时构建，重复 key 按 MEDI last-wins |
| **`AnyKey` 兜底注册** | ✅ 采用（同步） | `add_dbcontext_anykey`；精确 key 优先；有界 LRU（默认 256，`add_dbcontext_anykey_with_capacity`）；默认实体模型；异步 AnyKey 仍未提供 |
| **`AsyncFactoryContext` / 链式 async 环检测** | ✅ 采用底层能力 | `add_dbcontext_async` / `add_dbcontext_keyed_async` 使用 `async_singleton`；可选 `async_preflight`；EF 工厂不派生子任务 |
| **异步枚举语义 / 新诊断错误 / MSRV / 安全基线** | ✅ 文档与兼容 / 全量采用 | EF 不包装 `RdiError`；MSRV 1.95、`#![forbid(unsafe_code)]`、SECURITY、CI 加固保持 |

## 3. 设计说明

### 3.1 类型化 key：注册、解析、注入三处一致

```text
#[derive(DbContextKey)] enum Db { Read, Write }
        │
        ├─ IDbContextKey::name()  → "read" / "write"（实体模型 #[context("read")]）
        └─ From<Db> for ServiceKey → ServiceKey::new(Db::Read)（typed key）
```

`add_dbcontext_keyed(Db::Read, ...)` 与 `provider.get_owned_context(Db::Read)`、以及
`#[inject(owned, key = Db::Read)]` 使用同一个 typed key；实体发现仍用字符串 `name()`。
字符串 key（`"read"`）仍注册为 `String` key，与 typed enum key **不互通**，这与 rust-dix 0.10
的 key 类型相等语义一致。派生宏要求 enum 同时派生 `Debug + Clone + Copy + PartialEq + Eq + Hash`
（`ServiceKey::new` 的约束）。

### 3.2 AnyKey 的缓存与安全

- options/provider 以 `ServiceKey` 为键缓存：同一请求 key 跨 scope 复用连接池，不同 key
  独立；缓存是**有界 LRU**（默认 256，`add_dbcontext_anykey_with_capacity` 可调），避免
  key 基数导致内存无限增长；淘汰只移除缓存项，活跃 `DbContext` 仍持有 provider `Arc`。
- 动态 key 无 `#[context]` 字符串，因此强制 `use_default_entity_model()`；租户隔离必须由
  查询过滤器/应用层保证。
- **安全边界**：`ServiceKey` 只暴露 `Debug`/`Display`，不做 typed downcast。业务必须把
  请求 key 映射到已知租户配置（白名单/数据库表），禁止拼接 SQL/连接串。

### 3.3 异步 provider 初始化

- 注册模型：`async_singleton<AsyncDbContextOptions>`（`build_async` 初始化）+ 同步
  `scoped<DbContext>`（从容器取 options 并用已建好的 provider）。
- 好处：失败从“注册期 panic”变为类型化 `RdiError`；`build_async` 可与其他 async
  singleton 一起按依赖顺序初始化；`DbContext` 仍是同步 Scoped 服务，Handler 的
  `#[inject(owned)]` 不受影响。
- **async preflight**：`DbContextOptionsBuilder::async_preflight` 在连接池创建后、
  `build_async` 完成前执行异步自检（如 `SELECT 1`），失败中止启动；`configure` 本身仍是
  同步闭包。

## 4. 仍保留的边界与后续路线

1. **异步 AnyKey**（上游限制）：dix 0.10 明确未提供 `async_any_*`；动态 key 的 provider 在
   首次解析时同步构建。若未来 dix 提供 async AnyKey，再评估 `add_dbcontext_anykey_async`。
2. **`#[context]` 枚举化**（3.0 设计）：实体宏/元数据仍使用字符串 context key；typed key
   的 `name()` 是二者桥梁。
3. **生态**：`rust-webx` 升级到 rust-dix 0.10 后才能与本版本组合发布。
4. **效率**：枚举 key 已避免字符串分配；字符串 key 仍按 rust-dix 语义使用 `String` key。
