> **注**：本文档中 `IDbContext` trait 相关内容已过时。该 trait 已被移除，`DbContext` 现为具体上下文类型，通过 DI 注册为 `Arc<DbContext>`。

# Phase 2：多租户基础设施与线程安全重构方案

> 本计划回应两个核心诉求：(1) 讨论线程安全——REF 框架设计是否解决了多线程并发数据操作"跟踪结果污染导致被竞争应用保存"；(2) 按 EFCore 设计将 `add_dbcontext` 改为 Scoped 生命周期，并以此作为多租户隔离的基础设施。
>
> 全程遵循"从简实现"原则：能 10 行解决不写 50 行，能遍历 10 次不遍历 50 次。

---

## 一、线程安全分析（讨论）

### 1.1 直接回答用户的问题

> *"rust 中或者 REF 框架的设计是否解决了多线程并发数据操作跟踪结果污染，导致被竞争应用保存？"*

**当前答案是：没有解决。** REF 当前的设计存在两个层面的缺陷，使得"跟踪结果污染被竞争保存"真实可发生：

| 层面 | 现状 | 后果 |
|------|------|------|
| **生命周期** | `add_dbcontext` 用 `transient`（[di.rs:94](file:///d:/GitCode/RF/rust-ef/crates/core/src/di.rs#L94)、[di.rs:109](file:///d:/GitCode/RF/rust-ef/crates/core/src/di.rs#L109)、[di.rs:121](file:///d:/GitCode/RF/rust-ef/crates/core/src/di.rs#L121)） | 每次 DI 解析创建新实例 → 天然隔离，**但**用户一旦手动 `Arc<Mutex<DbContext>>` 共享，污染立即出现 |
| **save_changes 语义** | 遍历 `self.sets` 中**所有** DbSet，逐一调用 `saver.save()`，最后 `saver.clear()` 清空全部（[db_context.rs:470-509](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L470-L509)） | Thread A 调用 `save_changes()` 会把 Thread B 挂起在 `DbSet.entries` 里的变更一并提交，然后清空——B 的变更被"竞争应用保存"，且 B 完全无感知 |

### 1.2 污染的具体路径（双轨跟踪 Bug）

REF 当前存在**双轨跟踪**，两套数据源互不联动：

```
轨 1：DbSet.entries（Vec<TrackedEntry>）   ← save_changes 真实数据源
轨 2：DbContext.change_tracker             ← 拦截器 SaveChangesContext 数据源
```

- `DbSet::add()` 只写入 `DbSet.entries`（[db_set.rs:111-113](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_set.rs#L111-L113)），**从不**写入 `change_tracker`
- `save_changes()` 第 464 行 `SaveChangesContext::from_tracker(&self.change_tracker)` 从空的 `change_tracker` 构建 → 拦截器看到 0 条
- 但第 482 行 `saver.save()` 实际从 `db_set.tracked_by_state()` 读取 → 真实保存 N 条
- 第 504 行 `change_tracker.accept_all_changes()` 操作空 tracker，无实际效果

**后果**：拦截器（用户实现多租户/审计的钩子点）拿到的快照与实际提交内容不一致，任何基于拦截器的租户注入、审计逻辑都会失准。

### 1.3 EFCore 的解法与 REF 的对应

EFCore 的设计立场明确：
1. **DbContext 非线程安全**——这是有意为之，单个 DbContext 实例不应跨线程共享
2. **Scoped 生命周期**——每个请求/作用域一个实例，作用域结束自动释放
3. **单位工作（Unit of Work）隔离**——每个 DbContext 实例的 change tracker 独立

REF 应完全对齐：
- **DbContext 保持非线程安全**（不引入 `Arc<Mutex>` 包裹，那是反模式）
- **`add_dbcontext` 改 Scoped**——同一 `Scope` 内复用同一实例（单位工作），不同 `Scope` 隔离
- **从根 `ServiceProvider` 直接解析 scoped 退化为 transient**——这是 `rust-dicore` 的预期行为（[scope.rs:104-119](file:///C:/Users/lusid/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rust-dicore-0.3.2/src/scope.rs#L104-L119)：scoped 仅在 `Scope` 的 `scoped_cache` 内缓存），等价于"无作用域则每用一新实例"，天然防污染
- **无需改 `save_changes` 的遍历语义**——Scoped 保证了同作用域内只有一个调用者操作该实例，污染前提（多线程共享同一实例）被消除

> **结论**：线程安全不靠给 DbContext 加锁，而靠"Scoped 生命周期 + 作用域隔离"从根上杜绝跨线程共享同一跟踪状态。这是 EFCore 的立场，也是 REF 应采取的立场。

---

## 二、当前状态分析

### 2.1 DI 注册（`crates/core/src/di.rs`）

| 方法 | 当前 | 行号 |
|------|------|------|
| `add_dbcontext` | `self.transient(...)` | 94 |
| `add_dbcontext_keyed` | `self.keyed_transient(key, ...)` | 109 |
| `add_dbcontext_from_options` | `self.transient(...)` | 121 |

`rust-dicore` 已提供 `scoped(f)` 与 `keyed_scoped(k, f)`（collection.rs:35/67），`ServiceProvider::create_scope()` 可创建子作用域。无需新增依赖。

### 2.2 跟踪双轨问题

- 真实数据源：`DbSet.entries` → `tracked_by_state()`（[db_set.rs:213](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_set.rs#L213)）
- 拦截器数据源：`change_tracker`（空）→ `SaveChangesContext::from_tracker`（[interceptor.rs:53](file:///d:/GitCode/RF/rust-ef/crates/core/src/interceptor.rs#L53)）

### 2.3 查询过滤器覆盖缺口

`has_query_filter`（[model_builder.rs:164](file:///d:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L164)）目前仅覆盖 SELECT 路径（`apply_query_filter` [query.rs:779](file:///d:/GitCode/RF/rust-ef/crates/core/src/query.rs#L779)）。以下路径**完全绕过**过滤器：

| 路径 | 文件 | 问题 |
|------|------|------|
| UPDATE | [change_executor.rs:76-144](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs#L76-L144) | WHERE 仅含 PK + 并发令牌（`build_where_with_concurrency` 190-232） |
| DELETE | [change_executor.rs:148-188](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs#L148-L188) | 同上 |
| Navigation load | [navigation_loader.rs:37](file:///d:/GitCode/RF/rust-ef/crates/core/src/navigation_loader.rs#L37) | 裸 `SELECT * FROM {table} WHERE {fk} IN (...)` |

过滤器 SQL 复用所需的 `compile_bool_expr`（[query.rs:1573](file:///d:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1573)）与 `collect_bool_expr_values`（[query.rs:1605](file:///d:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1605)）当前是私有 `fn`，需改为 `pub(crate)`。

### 2.4 `IDbContext: Send + Sync` 约束

`IDbContext: Send + Sync`（[db_context.rs:389](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L389)）由 `rust-dicore` 的 `transient<T: ?Sized + Send + Sync + 'static>` 硬性要求。`scoped<T>` 同样要求 `Send + Sync`，故 DbContext 类型本身已满足（其字段均为 `Send+Sync`）。**该约束保留不变**——它约束的是"类型可跨线程传递"，而非"实例可跨线程共享"。

---

## 三、改造方案（5 个 Task）

> 顺序按依赖与见效速度排列。每个 Task 独立可验证。

### Task 1：Scoped 生命周期 + Scope API

**目标**：`add_dbcontext` 改 Scoped，提供 `create_dbcontext_scope()` 便捷 API。

**改动文件**：`crates/core/src/di.rs`

**改动 1 — 3 处注册换 scoped**：
```rust
// 第 94 行：transient → scoped
self.scoped(move |_| {
    let ctx = T::from_options(&options).expect("Failed to create DbContext");
    Arc::new(ctx) as Arc<dyn IDbContext>
})

// 第 109 行：keyed_transient → keyed_scoped
self.keyed_scoped(key, move |_| { ... })

// 第 121 行：transient → scoped
self.scoped(move |_| { ... })
```

**改动 2 — 新增 Scope 便捷 trait**（`di.rs` 末尾）：
```rust
/// Convenience trait for creating a scoped DbContext resolution scope.
pub trait DbContextScopeExt {
    /// Creates a new DI scope. Scoped DbContext instances resolved from
    /// this `Scope` are cached within it (unit-of-work isolation).
    fn create_dbcontext_scope(self: &Arc<Self>) -> rust_dicore::Scope;
}

impl DbContextScopeExt for ServiceProvider {
    fn create_dbcontext_scope(self: &Arc<Self>) -> rust_dicore::Scope {
        self.create_scope()
    }
}
```

**说明**：这只是对 `provider.create_scope()` 的语义化别名，1 行实现，便于用户发现用法。`rust-dicore` 已实现全部缓存逻辑，无需重复造轮子。

**行为约定**（写入文档）：
- 根 `ServiceProvider` 直接 `get::<dyn IDbContext>()`：退化为每次新实例（等价 transient，向后兼容）
- `provider.create_scope()` → `scope.get::<dyn IDbContext>()`：同作用域内复用同一实例

**测试**（新建 `crates/core/tests/scoped_lifecycle_tests.rs`）：
1. 同一 Scope 两次 `get::<dyn IDbContext>()` 返回同一实例（`Arc::ptr_eq`）
2. 不同 Scope 返回不同实例
3. 根 provider 直接解析，两次返回不同实例

---

### Task 2：双轨跟踪统一

**目标**：让拦截器的 `SaveChangesContext` 反映真实将提交的变更（来自 `DbSet.entries`），而非空的 `change_tracker`。

**根因**：`change_tracker` 与 `DbSet.entries` 两套独立存储。统一方式有两种——
- (A) 废弃 `change_tracker`，`save_changes` 直接从各 DbSet 聚合 entries 构建 `SaveChangesContext`
- (B) `DbSet::add()` 同时写入 `change_tracker`

**决策：采用 (A) 从简实现**。理由：
- `change_tracker` 的 `TrackerEntry` 只存 `type_id/type_name/state` 快照，无实体引用，对 save 流程无实际贡献（save 走 `DbSet.entries`）
- (B) 要在每次 `add/attach/update/remove_at` 都同步双写，易遗漏，违反"从简"
- (A) 只改 `save_changes` 一处构建逻辑，10 行内完成

**改动文件**：`crates/core/src/db_context.rs`（`save_changes` 第 464 行附近）

**改动**：增加一个私有聚合函数，从 `self.sets` 各 DbSet 聚合 `EntityEntry` 视图：
```rust
impl DbContext {
    /// Builds a SaveChangesContext from the actual pending entries across
    /// all DbSets (the real save data source), not from the (legacy, empty)
    /// change_tracker. This keeps interceptor snapshots consistent with
    /// what will actually be committed.
    fn build_save_context(&self) -> SaveChangesContext {
        let mut entries: Vec<EntityEntry> = Vec::new();
        // type-erased iteration via savers is not possible for reading typed
        // entries; instead expose DbSet entry counts through a small helper.
        // See implementation note below.
        ...
    }
}
```

**实现难点与从简解法**：`DbSet<T>` 是泛型，`self.sets` 是 `Box<dyn Any>`，无法直接迭代出 `EntityEntry`（需要 `T` 类型）。从简方案：在 `ErasedSetOps` trait 增加一个 `collect_entries(&self, raw_set: &dyn Any) -> Vec<EntityEntryView>` 方法，由 `SetOps<E>` 实现把 `tracked_by_state(Added/Modified/Deleted)` 转成无类型视图。

新增轻量视图类型（避免暴露内部 `TrackedEntry`）：
```rust
// interceptor.rs 或 tracking.rs
#[derive(Debug, Clone)]
pub struct EntityEntryView {
    pub type_name: &'static str,
    pub state: EntityState,
}
```

`SetOps<E>::collect_entries` 遍历 `db_set.entries` 一次即可（不额外遍历），把每个 `TrackedEntry` 映射为 `EntityEntryView`。`SaveChangesContext` 改为持有 `Vec<EntityEntryView>` 并据此统计 `added/modified/deleted_count`。

**改动文件**：
- `crates/core/src/interceptor.rs`：`SaveChangesContext` 字段类型 `EntityEntry` → `EntityEntryView`，新增 `from_views(views: Vec<EntityEntryView>)` 构造
- `crates/core/src/db_context.rs`：`ErasedSetOps` 加 `fn collect_entries(...)`；`save_changes` 第 464 行改用 `build_save_context()`
- `crates/core/src/tracking.rs`：新增 `EntityEntryView`（若不放 interceptor.rs）

**测试**：`add` 一个实体后，拦截器 `on_saving` 收到的 `ctx.entries().len()` == 1（之前为 0）。

---

### Task 3a：ChangeExecutor UPDATE/DELETE 应用查询过滤器

**目标**：UPDATE/DELETE 的 WHERE 子句追加查询过滤器（如 `tenant_id = ?`），防止跨租户修改/删除。

**改动文件**：
- `crates/core/src/query.rs`：`compile_bool_expr`、`collect_bool_expr_values` 由私有 `fn` 改 `pub(crate) fn`
- `crates/core/src/change_executor.rs`：`execute_updates` / `execute_deletes` 增加 `query_filter: Option<&BoolExpr>` 参数

**改动逻辑**（`execute_updates` 为例，`execute_deletes` 同理）：
```rust
pub async fn execute_updates<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    entities: &[(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)],
    query_filter: Option<&BoolExpr>,   // 新增
) -> EFResult<usize>
```

在 `build_where_with_concurrency` 返回的 `where_clause` 后追加：
```rust
let mut where_clause = base_where;
let mut extra_params = base_params;
if let Some(filter) = query_filter {
    let mut idx = extra_params.len() + 1;
    let filter_sql = compile_bool_expr(filter, gen, &mut idx);
    extra_params.extend(collect_bool_expr_values(filter));
    where_clause = format!("({}) AND ({})", where_clause, filter_sql);
}
```

**调用方**：`save_one_set`（[db_context.rs:534](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L534)）需把 DbSet 的 `query_filter` 透传。`save_one_set` 增加 `query_filter: Option<&BoolExpr>` 参数，`ErasedSetOps::save` 从 `DbSet.query_filter` 取值传入。

**从简**：复用现有 `compile_bool_expr`/`collect_bool_expr_values`，不引入新 SQL 构建函数；过滤器与 PK/并发令牌用 AND 连接，遍历次数不变。

**测试**：设置 `has_query_filter::<T>(tenant_id == 1)`，UPDATE 一个 `tenant_id=2` 的实体应触发并发冲突（0 rows affected）或被 WHERE 过滤掉。

---

### Task 3b：NavigationLoader 应用查询过滤器

**目标**：导航加载的 `SELECT * FROM related WHERE fk IN (...)` 追加过滤器，防止加载跨租户关联实体。

**改动文件**：`crates/core/src/navigation_loader.rs`

**新增 trait**（同文件或 `query.rs`）：
```rust
/// Resolves the query filter for a related entity type, used by
/// NavigationLoader to scope secondary queries (e.g. multi-tenant).
pub trait QueryFilterResolver {
    fn filter_for(&self, type_id: TypeId) -> Option<&BoolExpr>;
}
```

**改动**：`load_includes`（[navigation_loader.rs:37](file:///d:/GitCode/RF/rust-ef/crates/core/src/navigation_loader.rs#L37)）及 `load_scalar_navigation`/`load_many_to_many` 增加 `filter_resolver: Option<&dyn QueryFilterResolver>` 参数。在构造 `SELECT ... WHERE fk IN (...)` 后追加 `AND (filter_sql)`。

**从简**：过滤器解析用 trait 而非泛型，避免 `load_includes` 被泛型化扩散到所有调用方。默认传 `None` 保持现状行为，DbContext 调用时传入一个实现了 `QueryFilterResolver` 的闭包包装。

**调用方**：DbContext 在调用 `load_includes` 处构造 resolver（从 `model_builder.get_query_filter` 取）。如果当前无调用点（load_includes 由 DbSet query 触发），则让 DbSet 持有自身 filter 即可——多数导航加载的 related_table 过滤器需 DbContext 提供，故 DbContext 实现 `QueryFilterResolver` 是最简路径。

**测试**：父实体有跨租户 FK 指向另一租户子实体，加载 navigation 后子实体为空（被过滤器拦截）。

---

### Task 3c：IgnoreQueryFilters 逃逸出口

**目标**：提供管理/跨租户查询绕过过滤器的能力（对齐 EFCore `IgnoreQueryFilters`）。

**改动文件**：`crates/core/src/db_set.rs`、`crates/core/src/query.rs`

**改动**：DbSet 新增方法返回一个不应用 filter 的 QueryBuilder：
```rust
impl<T> DbSet<T> {
    /// Returns a query builder that bypasses the configured query filter.
    /// Use for administrative / cross-tenant queries.
    pub fn query_ignore_filters(&self) -> QueryBuilder<T> {
        let mut q = self.query();
        q = q.apply_query_filter(BoolExpr::Raw("1=1".into())); // 见下注
        q
    }
}
```

> **注**：实际实现需 QueryBuilder 支持"跳过 filter"标志位而非注入 `1=1`。最简方式是 `QueryBuilder` 增加一个 `ignore_filters: bool` 字段，`to_list` 等终态方法在 `apply_query_filter` 处判断该标志跳过。约 5 行改动。

**测试**：设置过滤器后，`query_ignore_filters().to_list()` 返回全部行（含其他租户）。

---

### Task 4：拦截器变更钩子（on_inserting/on_updating/on_deleting）

**目标**：提供保存前可变钩子，让用户实现"框架提供能力"（如自动注入 `tenant_id`、审计字段），而非框架写死租户逻辑。

**改动文件**：`crates/core/src/interceptor.rs`

**新增可变条目视图**：
```rust
/// Mutable view of an entity about to be persisted, exposed to
/// interceptors so they can modify scalar properties (e.g. set
/// tenant_id, audit timestamps) before SQL is generated.
pub struct MutatingEntityEntry<'a> {
    pub type_name: &'a str,
    pub state: EntityState,
    pub values: &'a mut HashMap<String, DbValue>,
}
```

**新增 trait 方法**（`ISaveChangesInterceptor`，默认空实现）：
```rust
async fn on_inserting(&self, _entry: &mut MutatingEntityEntry<'_>) -> EFResult<()> { Ok(()) }
async fn on_updating(&self, _entry: &mut MutatingEntityEntry<'_>) -> EFResult<()> { Ok(()) }
async fn on_deleting(&self, _entry: &mut MutatingEntityEntry<'_>) -> EFResult<()> { Ok(()) }
```

**调用点**：在 `save_one_set` 构建 `added/modified/deleted` Vec 之后、调用 `ChangeExecutor::execute_*` 之前，逐条调用对应钩子。

**从简决策**：
- 钩子接收 `&mut MutatingEntityEntry`，修改 `values` 后由 `ChangeExecutor` 读取——但 `ChangeExecutor` 当前直接调 `entity.snapshot()`，不读 `values`。
- **最简实现**：钩子直接接收 `&mut E`（泛型）不现实（trait object 安全）。故采用 `MutatingEntityEntry` 持有 `&mut HashMap<String, DbValue>`，`SetOps<E>` 在调用钩子时把 entity 的 snapshot 可变引用传出，钩子改完写回 entity。
- 这意味着 `ErasedSetOps` 需新增 `fn mutate_entries(&self, raw_set, pipeline, state)` 之类方法。考虑到复杂度上升，**本 Task 4 标记为可延后**：若 Task 1-3 + 文档已满足"多租户基础设施"目标，Task 4 可推到 Phase 3。

**建议**：先完成 Task 1/2/3/5，Task 4 视实施后评估。多租户写入隔离在 Task 3a 已由 UPDATE/DELETE 过滤器覆盖；INSERT 的 `tenant_id` 注入可由用户在 `add()` 前手动设置（框架提供 `query_filter` 能力即满足"提供能力而非写死"）。

---

### Task 5：并发安全文档

**目标**：明确记录线程安全契约，避免用户误用。

**改动文件**：
1. `crates/core/src/db_context.rs` 模块文档顶部增加"线程安全"小节
2. `crates/core/src/di.rs` 模块文档增加"Scoped 生命周期"说明
3. 新建 `docs/rust-ef/03-advanced/multi-tenancy-foundation.md`

**文档要点**：
- DbContext 非线程安全，禁止跨线程共享同一实例
- 使用 `create_dbcontext_scope()` 创建作用域，作用域内复用实例
- 多租户：用 `has_query_filter` 注册租户隔离谓词，框架自动应用于 SELECT/UPDATE/DELETE/Navigation
- 跨租户管理查询用 `query_ignore_filters()`
- 反模式：`Arc<Mutex<DbContext>>` 共享会导致跟踪污染

---

## 四、假设与决策

| # | 决策 | 理由 |
|---|------|------|
| D1 | DbContext 保持非线程安全 | 对齐 EFCore；加锁是反模式，Scoped 隔离才是正解 |
| D2 | `add_dbcontext` 改 Scoped，根 provider 解析退化为 transient | `rust-dicore` 既有行为，向后兼容 |
| D3 | 双轨统一采用方案 (A)：从 DbSet.entries 构建 SaveChangesContext | 只改一处，避免双写遗漏 |
| D4 | 不改 `save_changes` 遍历所有 DbSet 的语义 | Scoped 自然消除污染前提 |
| D5 | 查询过滤器复用 `compile_bool_expr`/`collect_bool_expr_values`（改 `pub(crate)`） | 不重复造 SQL 构建逻辑 |
| D6 | NavigationLoader 用 `QueryFilterResolver` trait 解析过滤器 | 避免泛型扩散 |
| D7 | Task 4（拦截器变更钩子）可延后至 Phase 3 | Task 3a 已覆盖写入隔离；INSERT 注入可用户手动 |
| D8 | 保留 `IDbContext: Send + Sync` 约束 | `rust-dicore` 硬性要求；约束类型可传递，不约束实例可共享 |

---

## 五、验证步骤

### 5.1 单元/集成测试

| Task | 测试文件 | 验证点 |
|------|----------|--------|
| 1 | `tests/scoped_lifecycle_tests.rs` | 同 Scope 同实例、异 Scope 异实例、根解析退化 |
| 2 | `tests/tracking_consistency_tests.rs` | add 后拦截器看到 1 条；save 后 accept_all 清空 |
| 3a | `tests/query_filter_exec_tests.rs` | UPDATE/DELETE 跨租户行触发冲突 |
| 3b | `tests/query_filter_nav_tests.rs` | 跨租户导航加载被过滤 |
| 3c | `tests/ignore_query_filters_tests.rs` | `query_ignore_filters` 返回全量行 |
| 4 | （若实施）`tests/interceptor_mutation_tests.rs` | `on_inserting` 修改的值被持久化 |

### 5.2 回归基线

- 现有 106 个测试全过（含 MySQL、auto-registration、linq、production）
- `cargo clippy --workspace --all-targets -- -D warnings` 清洁
- `cargo fmt --all --check` 清洁
- blog-example 8 步全过

### 5.3 实施顺序与每步验证

```
Task 1 (Scoped)        → cargo test scoped_lifecycle + 全量回归
Task 2 (双轨统一)      → cargo test tracking_consistency + 全量回归
Task 3a (Exec 过滤器)  → 改 pub(crate) + cargo check + 过滤器测试
Task 3b (Nav 过滤器)   → QueryFilterResolver + 过滤器测试
Task 3c (IgnoreFilters)→ 独立测试
Task 5 (文档)          → 最后
（Task 4 视评估）
```

每完成一个 Task 运行 `cargo test --workspace` 确保无回归，再进入下一个。

---

## 六、文件变更清单

| 文件 | 操作 | Task |
|------|------|------|
| `crates/core/src/di.rs` | 改 3 处 transient→scoped + 新增 `DbContextScopeExt` | 1 |
| `crates/core/tests/scoped_lifecycle_tests.rs` | 新建 | 1 |
| `crates/core/src/db_context.rs` | `ErasedSetOps` 加 `collect_entries`；`save_changes` 用 `build_save_context`；`save_one_set` 加 `query_filter` 参数；`ErasedSetOps::save` 透传 filter | 2, 3a |
| `crates/core/src/interceptor.rs` | `SaveChangesContext` 改用 `EntityEntryView`；新增 `from_views` | 2 |
| `crates/core/src/tracking.rs` | 新增 `EntityEntryView` | 2 |
| `crates/core/tests/tracking_consistency_tests.rs` | 新建 | 2 |
| `crates/core/src/query.rs` | `compile_bool_expr`/`collect_bool_expr_values` 改 `pub(crate)`；`QueryBuilder` 加 `ignore_filters` 字段 | 3a, 3c |
| `crates/core/src/change_executor.rs` | `execute_updates`/`execute_deletes` 加 `query_filter` 参数 | 3a |
| `crates/core/src/navigation_loader.rs` | `load_includes` 及子函数加 `filter_resolver` 参数 | 3b |
| `crates/core/src/db_set.rs` | 新增 `query_ignore_filters()` | 3c |
| `crates/core/tests/query_filter_*.rs` | 新建（3a/3b/3c 各一） | 3 |
| `docs/rust-ef/03-advanced/multi-tenancy-foundation.md` | 新建 | 5 |
