> **注**：本文档中 `IDbContext` trait 相关内容已过时。该 trait 已被移除，`DbContext` 现为具体上下文类型，通过 DI 注册为 `Arc<DbContext>`。

# Phase 2: 多租户基础设施与并发安全

## Context

Phase 1 完成了 inventory 自动注册和 `save_one_set` 配置化元数据传递。深入分析后发现两个架构问题：

1. **Scoped 生命周期缺失**：`add_dbcontext` 使用 transient，无法实现 unit-of-work 模式。EFCore 用 Scoped（每请求一个实例），rust-dicore 已支持但未使用。
2. **双轨跟踪 Bug**：`DbSet.entries` 是 save\_changes 的真实数据源，但拦截器的 `SaveChangesContext` 从 `change_tracker` 构建（经常为空）。`DbSet::add()` 不写入 `change_tracker`，导致拦截器看到 0 条而实际保存 N 条。

此外，查询过滤器仅覆盖 SELECT，**导航加载和 save\_changes 的 UPDATE/DELETE 完全绕过过滤器**，存在跨租户数据泄露。

**设计原则**：框架提供能力不写死租户逻辑；从简实现，10 行能解决的绝不写 50 行。

## Task 1: Scoped 生命周期

**改动**：[di.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/di.rs) — 3 处 `transient`→`scoped`，`keyed_transient`→`keyed_scoped`

```rust
// add_dbcontext: transient → scoped
self.scoped(move |_| { ... })
// add_dbcontext_keyed: keyed_transient → keyed_scoped
self.keyed_scoped(key, move |_| { ... })
// add_dbcontext_from_options: transient → scoped
self.scoped(move |_| { ... })
```

**Scope API**：在 di.rs 新增 trait 方法，让用户能正确创建 Scope：

```rust
/// Creates a service scope for scoped DbContext resolution.
/// Within a scope, multiple get() calls return the same DbContext instance.
fn create_dbcontext_scope(&self) -> ServiceScope;
```

实际委托给 `rust_dicore::ServiceProvider::create_scope()`。用户模式：

```rust
let scope = services.create_dbcontext_scope();
let ctx = scope.get::<dyn IDbContext>()?; // 同一 scope 内同一实例
```

**注意**：直接从根 ServiceProvider 解析 scoped 服务退化为 transient 行为。必须通过 Scope 解析才缓存。文档注明。

## Task 2: 双轨跟踪统一

**问题**：`DbSet::add()` 只写入 `DbSet.entries`，不写入 `DbContext.change_tracker`。拦截器 `SaveChangesContext::from_tracker()` 从 change\_tracker 构建，看到 0 条。

**改动**：[db\_set.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_set.rs) — `add()`/`attach()`/`update()`/`remove()` 同步写入 change\_tracker

关键：DbSet 不持有 change\_tracker 的引用（避免循环引用）。改为让 DbContext 在 `save_changes` 中从 DbSet.entries 构建 `SaveChangesContext`，而非从 change\_tracker。

**最简方案**（不改 DbSet 签名）：

* [interceptor.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/interceptor.rs) — `SaveChangesContext::from_tracker` 改为接受 entries 计数

* [db\_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs) — `save_changes` 中从各 DbSet 的 `tracked_by_state` 统计 added/modified/deleted 数量，构建上下文

```rust
// save_changes 中，替代 from_tracker(&self.change_tracker)
let mut added = 0; let mut modified = 0; let mut deleted = 0;
for type_id in &type_ids {
    let set = self.sets.get(type_id).unwrap();
    // 通过 ErasedSetOps 增加 count 方法或直接 downcast
}
let save_ctx = SaveChangesContext { added, modified, deleted, .. };
```

**备选方案**（更彻底但改动大）：`DbContext::set::<T>()` 创建 DbSet 时传入 `&ChangeTracker` 引用，让 DbSet.add() 直接写入。但 DbSet 当前是 owned struct，引入引用需改生命周期，复杂度高。不采用。

## Task 3: 查询过滤器全覆盖

### 3a: ChangeExecutor UPDATE/DELETE 应用过滤器

**改动**：

* [query.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/query.rs) — `compile_bool_expr`、`collect_bool_expr_values` 改为 `pub(crate)`

* [change\_executor.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs) — `execute_updates`/`execute_deletes` 增加 `query_filter: Option<&BoolExpr>` 参数，在 WHERE 后 AND

* [db\_set.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_set.rs) — 增加 `pub(crate) fn query_filter(&self) -> Option<&BoolExpr>`

* [db\_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs) — `save_one_set` 从 DbSet 取 filter 传给 ChangeExecutor

### 3b: NavigationLoader 应用过滤器

**改动**：

* [navigation\_loader.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/navigation_loader.rs) — `load_includes` 增加 `filter_resolver: Option<&dyn QueryFilterResolver>` 参数

* 新增 trait（放 query.rs）：

```rust
pub trait QueryFilterResolver: Send + Sync {
    fn resolve_filter(&self, type_id: &TypeId) -> Option<BoolExpr>;
}
```

* DbContext 实现 QueryFilterResolver

* DbSet 持有 `Option<Arc<dyn QueryFilterResolver>>`，query() 时传给 QueryBuilder

**自引用**：将 ModelBuilder 的过滤器配置提取为 `Arc<HashMap<TypeId, BoolExpr>>`，DbSet 持有克隆。

### 3c: IgnoreQueryFilters

**改动**：

* [query.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/query.rs) — QueryState 增加 `ignore_query_filters: bool`；`to_sql_with` 中跳过过滤器

* QueryBuilder 增加 `ignore_query_filters(mut self) -> Self`

* DbSet 增加 `query_ignore_filters(&self) -> QueryBuilder<T>`

**简化**：不重构为延迟应用。在 `DbSet::query()` 时仍立即 apply，但额外存一份 filter 引用。`ignore_query_filters` 时用新 QueryBuilder 绕过 `apply_query_filter`：

```rust
pub fn query_ignore_filters(&self) -> QueryBuilder<T> {
    let mut qb = QueryBuilder::with_provider(self.table_name.clone(), Arc::clone(&self.provider?));
    // 不调 apply_query_filter
    qb
}
```

## Task 4: 拦截器变更钩子

**改动**：

* [interceptor.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/interceptor.rs) — 新增 `MutatingEntityEntry` + 3 个默认空钩子：

```rust
pub struct MutatingEntityEntry {
    pub type_id: TypeId, pub type_name: String, pub state: EntityState,
    pub current_values: HashMap<String, DbValue>,
}
async fn on_inserting(&self, _e: &mut [MutatingEntityEntry]) -> EFResult<()> { Ok(()) }
async fn on_updating(&self, _e: &mut [MutatingEntityEntry]) -> EFResult<()> { Ok(()) }
async fn on_deleting(&self, _e: &mut [MutatingEntityEntry]) -> EFResult<()> { Ok(()) }
```

* [change\_executor.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs) — 新增 `execute_*_with_snapshots` 接受 `&[MutatingEntityEntry]`

* [db\_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs) — `save_one_set` 先提取快照→调钩子→执行

**ErasedSetOps::save** 增加 `interceptors: &InterceptorPipeline` 参数。

## Task 5: 并发安全文档

不改代码（`&mut self` + Scoped 已提供编译期保护）。仅文档：

* [db\_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs) 模块文档增加线程安全模型

* [di.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/di.rs) `add_dbcontext` 文档注明 Scoped + 禁止跨 Scope 共享

* 新增 `docs/rust-ef/12-multi-tenancy/multi-tenancy-foundation.md`

## 实施顺序

```
1. Task 1 (Scoped)       ← 3 行改动 + Scope API，最快见效
2. Task 2 (双轨统一)     ← 正确性 Bug 修复
3. Task 3a (Exec 过滤器)  ← 依赖 pub(crate) 函数
4. Task 3b (Nav 过滤器)   ← 依赖 QueryFilterResolver
5. Task 3c (IgnoreFilters)← 独立
6. Task 4 (拦截器钩子)    ← 重构 save 流程
7. Task 5 (文档)         ← 最后
```

## 测试

* **Task 1**: 创建 Scope，两次 `get::<dyn IDbContext>()` 返回同一实例；不同 Scope 返回不同实例

* **Task 2**: `add()` 后拦截器 `on_saving` 看到正确的 added\_count

* **Task 3a**: 注册过滤器，save\_changes 的 UPDATE/DELETE 仅影响过滤范围内行

* **Task 3b**: 注册过滤器 + include，验证不加载被过滤的相关实体

* **Task 3c**: `query_ignore_filters()` 返回全部行

* **Task 4**: `on_inserting` 修改的值出现在 INSERT 中

## 验证

1. `cargo check/clippy/fmt` — 全部通过
2. `cargo test --workspace` — 全部测试通过
3. `cargo run -p rust-ef-blog-example` — 无回归

