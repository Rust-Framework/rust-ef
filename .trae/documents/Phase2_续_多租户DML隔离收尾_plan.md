> **注**：本文档中 `IDbContext` trait 相关内容已过时。该 trait 已被移除，`DbContext` 现为具体上下文类型，通过 DI 注册为 `Arc<DbContext>`。

# Phase 2（续）：多租户 DML 隔离收尾与线程安全契约

> 本计划回应 `/plan` 指令中的两个核心诉求：
> 1. **讨论线程安全**——REF 框架设计是否解决了多线程并发数据操作"跟踪结果污染导致被竞争应用保存"
> 2. **规划 Phase 2 剩余工作**——以"多租户支持"为重点，收尾前序会话遗留的断点
>
> 全程遵循"从简实现"原则：能 10 行解决不写 50 行，能遍历 10 次不遍历 50 次。

---

## 一、线程安全分析（讨论）

### 1.1 直接回答

> *"rust 中或者 REF 框架的设计是否解决了多线程并发数据操作跟踪结果污染，导致被竞争应用保存？"*

**答案是：现在已解决（Task 1 Scoped 生命周期已落地）。** 本次会话的代码探索已验证：

| 防线 | 当前状态 | 验证位置 |
|------|----------|----------|
| **Scoped 生命周期** | `add_dbcontext` 用 `scoped`（非 `transient`） | [di.rs:107](file:///d:/GitCode/RF/rust-ef/crates/core/src/di.rs#L107) |
| **作用域隔离** | `DbContextScopeExt::create_dbcontext_scope()` 可用 | [di.rs:155-186](file:///d:/GitCode/RF/rust-ef/crates/core/src/di.rs#L155-L186) |
| **根 provider 退化** | 从根 `ServiceProvider` 解析 scoped → 退化为 transient（每次新实例） | rust-dicore scope.rs:104-119 |
| **跟踪一致性** | 拦截器 `SaveChangesContext` 从 `DbSet.entries` 构建（非空 `change_tracker`） | [db_context.rs:357-365](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L357-L365) |

### 1.2 污染路径为何被消除

前序会话存在两层缺陷，**现均已修复**：

**缺陷一：生命周期（已修复）**
- 旧：`transient` 注册——天然隔离但无作用域复用语义
- 新：`scoped` 注册——同一 `Scope` 内复用同一实例（单位工作），不同 `Scope` 隔离。从根 provider 解析退化为每次新实例。
- **结论**：用户无需 `Arc<Mutex<DbContext>>` 共享——这是反模式。正确做法是每个请求/操作创建一个 `Scope`。

**缺陷二：双轨跟踪（已修复）**
- 旧：`DbSet.entries`（save_changes 真实数据源）与 `DbContext.change_tracker`（拦截器数据源）互不联动；`DbSet::add()` 只写 entries 不写 tracker → 拦截器看到 0 条
- 新：`build_save_context()` 从 `DbSet.entries` 聚合 `EntityEntryView` → `SaveChangesContext::from_views()` → 拦截器看到真实将提交的变更
- **结论**：拦截器（用户实现多租户/审计的钩子点）的快照与实际提交内容一致。

### 1.3 EFCore 对齐与 REF 立场

| 维度 | EFCore | REF（当前） |
|------|--------|-------------|
| DbContext 线程安全 | 非线程安全（有意为之） | ✅ 对齐——不引入 `Arc<Mutex>` 包裹 |
| 生命周期 | Scoped | ✅ 对齐——`add_dbcontext` 用 `scoped` |
| 单位工作隔离 | 每个 Scope 一个实例 | ✅ 对齐——`create_dbcontext_scope()` |
| save_changes 遍历语义 | 遍历所有 DbSet | ✅ 保留——Scoped 自然消除污染前提（同作用域内只有一个调用者） |
| `IDbContext: Send + Sync` | — | ✅ 保留——rust-dicore 硬性要求；约束类型可传递，不约束实例可共享 |

> **线程安全结论**：不靠给 DbContext 加锁，而靠"Scoped 生命周期 + 作用域隔离"从根上杜绝跨线程共享同一跟踪状态。这是 EFCore 的立场，也是 REF 已采取的立场。**防线已就位，无需再改 DI/跟踪层。**

### 1.4 剩余的隔离缺口（本次计划重点）

线程安全（防污染）已解决，但**多租户数据隔离**仍有 DML 断点：

| 路径 | 当前 | 风险 |
|------|------|------|
| SELECT | ✅ 已应用 query_filter | — |
| UPDATE | ❌ `save_one_set` 未透传 filter | 跨租户修改 |
| DELETE | ❌ 同上 | 跨租户删除 |
| INSERT | ⚠️ tenant_id 需用户在 `add()` 前设置 | 框架提供 `query_filter` 能力，不写死注入逻辑 |
| Navigation load | ❌ 裸 SQL `SELECT * FROM related WHERE fk IN (...)` | 跨租户关联实体泄漏 |
| 跨租户管理查询 | ❌ 无逃逸出口 | 管理员无法查全量 |

> ChangeExecutor 已接收 `query_filter` 参数并 AND 进 WHERE（[change_executor.rs:124-129](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs#L124-L129)、[195-200](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs#L195-L200)），但管线在 `save_one_set` 处断裂——**这就是本次计划要收尾的断点**。

---

## 二、当前状态基线（已验证）

### 2.1 已完成（前序会话落盘，本次探索确认）

| 组件 | 状态 | 证据 |
|------|------|------|
| Scoped 生命周期 | ✅ | di.rs:107 `scoped`、125 `keyed_scoped`；`DbContextScopeExt` trait 已定义 |
| `DbContextScopeExt` prelude 导出 | ✅ | lib.rs:55 |
| `EntityEntryView` 类型 | ✅ | tracking.rs:33-45（`type_id`/`type_name`/`state`） |
| `ErasedSetOps::collect_entries` | ✅ | db_context.rs:166、214-228 |
| `build_save_context()` | ✅ | db_context.rs:357-365 |
| `SaveChangesContext::from_views` | ✅ | interceptor.rs:53-73；`from_tracker` 已移除 |
| `query.rs` helper `pub(crate)` | ✅ | query.rs:1573、1605 |
| `execute_updates`/`execute_deletes` 接收 `query_filter` | ✅ | change_executor.rs:81-89、166-174 |
| filter AND 进 WHERE | ✅ | change_executor.rs:124-129、195-200 |
| `scoped_lifecycle_tests.rs` | ✅ | 3 测试 |
| `tracking_consistency_tests.rs` | ✅ | 1 测试 |

### 2.2 未完成（本次计划目标）

| 断点 | 位置 | 问题 |
|------|------|------|
| DbSet 缺 `query_filter()` getter | [db_set.rs:68](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_set.rs#L68) | `query_filter` 字段私有，`save_one_set` 无法读取 |
| `save_one_set` 未透传 filter | [db_context.rs:600](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L600)、[603](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L603) | 调用 `execute_updates`/`execute_deletes` 时传 `None`（缺第 4 参数）——实际编译应已报错，需确认 |
| `query_filter_exec_tests.rs` | 不存在 | DML 端过滤器端到端测试缺失 |
| NavigationLoader 过滤器 | [navigation_loader.rs:117-122](file:///d:/GitCode/RF/rust-ef/crates/core/src/navigation_loader.rs#L117-L122)、[150-155](file:///d:/GitCode/RF/rust-ef/crates/core/src/navigation_loader.rs#L150-L155)、[266-271](file:///d:/GitCode/RF/rust-ef/crates/core/src/navigation_loader.rs#L266-L271) | 3 处裸 SQL 无 filter |
| IgnoreQueryFilters 逃逸出口 | — | 无 |
| 并发安全文档 | — | 无 |

---

## 三、改造方案（4 个 Task）

> 顺序按依赖与见效速度排列。Task 3a 是 CRITICAL（写隔离），3c 最简（5 行），3b 中等（导航读隔离），5 文档收尾。

### Task 3a-finish：收尾 ChangeExecutor → save_one_set 过滤器管线

**目标**：闭合 DML 过滤器管线的最后断点，让 UPDATE/DELETE 的 WHERE 子句包含 query_filter。

**改动文件**：
1. `crates/core/src/db_set.rs`
2. `crates/core/src/db_context.rs`
3. 新建 `crates/core/tests/query_filter_exec_tests.rs`

**改动 1 — DbSet 新增 getter**（[db_set.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_set.rs)，`set_query_filter` 附近）：
```rust
/// Returns the configured query filter, if any. Used by `save_one_set`
/// to apply tenant isolation to UPDATE/DELETE WHERE clauses.
pub(crate) fn query_filter(&self) -> Option<&BoolExpr> {
    self.query_filter.as_ref()
}
```
> 1 行实现。`query_filter` 字段已存在（第 68 行），只是加个 getter。`pub(crate)` 限制在 crate 内可见，不污染公共 API。

**改动 2 — `save_one_set` 读取并透传 filter**（[db_context.rs:569-606](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L569-L606)）：

签名不变（仍接收 `db_set: &mut DbSet<E>`），内部读取 filter 后传入：

```rust
pub async fn save_one_set<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
    meta: &EntityTypeMeta,
) -> EFResult<(usize, usize, usize)>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues + IFromRow,
{
    // 读取 filter（不可变借用，与后续 tracked_by_state 的 &self 兼容）
    let query_filter = db_set.query_filter();

    let added: Vec<(&E, &EntityTypeMeta)> = db_set
        .tracked_by_state(crate::entity::EntityState::Added)
        .into_iter()
        .map(|(e, _)| (e, meta))
        .collect();
    let modified: Vec<(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)> = /* ... */;
    let deleted: Vec<(&E, &EntityTypeMeta, Option<&HashMap<String, DbValue>>)> = /* ... */;

    // ...
    if !modified.is_empty() {
        uc = ChangeExecutor::execute_updates(conn, provider, &modified, query_filter).await?;
    }
    if !deleted.is_empty() {
        dc = ChangeExecutor::execute_deletes(conn, provider, &deleted, query_filter).await?;
    }
    Ok((ac, uc, dc))
}
```

**借用安全性分析**：
- `db_set.query_filter()` 返回 `Option<&BoolExpr>`，借用 `db_set.query_filter` 字段
- `db_set.tracked_by_state(...)` 返回 `Vec<(&E, ...)>`，借用 `db_set.entries` 字段
- 两者都是 `&self` 不可变借用，Rust 允许同一值的多个不可变借用共存
- `tracked_by_state` 返回的 `Vec` 持有 `&E` 引用（借用 entries），`query_filter` 持有 `&BoolExpr` 引用（借用 query_filter 字段）——不冲突
- `execute_updates`/`execute_deletes` 接收 `Option<&BoolExpr>`，与 `&modified`/`&deleted`（借用 entries）无重叠

**改动 3 — 验证编译**：
- `SetOps<E>::save` 中 `save_one_set(conn, provider, db_set, meta)` 调用不变——`db_set` 是 `&mut DbSet<E>`，`query_filter()` 接收 `&self`，自动 reborrow
- 现有 6 处 `save_one_set` 调用点（sqlite_crud_tests.rs 等）无需修改——签名未变

**测试**（新建 `crates/core/tests/query_filter_exec_tests.rs`）：

验证设置 `has_query_filter::<T>(tenant_id == 1)` 后：
1. UPDATE 一个 `tenant_id=2` 的实体 → 0 rows affected（被 WHERE 过滤）
2. DELETE 一个 `tenant_id=2` 的实体 → 0 rows affected
3. UPDATE 一个 `tenant_id=1` 的实体 → 正常执行

```rust
// 测试骨架
#[tokio::test]
async fn update_across_tenant_filtered_out() {
    // 1. 建表 + 插入 tenant_id=2 的行
    // 2. ctx.model().has_query_filter::<Entity>(linq!(|b: Entity| b.tenant_id == 1))
    // 3. attach tenant_id=2 的实体，修改某字段
    // 4. save_changes() → updated == 0（filter 拦截）
}
```

---

### Task 3c：IgnoreQueryFilters 逃逸出口

**目标**：提供管理/跨租户查询绕过过滤器的能力（对齐 EFCore `IgnoreQueryFilters`）。

**改动文件**：`crates/core/src/db_set.rs`

**改动**：DbSet 新增方法，返回一个**不应用** filter 的 QueryBuilder：
```rust
impl<T: IEntityType + IEntitySnapshot> DbSet<T> {
    /// Returns a query builder that bypasses the configured query filter.
    /// Use for administrative / cross-tenant queries.
    pub fn query_ignore_filters(&self) -> QueryBuilder<T> {
        let mut qb = match &self.provider {
            Some(p) => QueryBuilder::with_provider(&self.table_name, p.clone()),
            None => QueryBuilder::new(&self.table_name),
        };
        // 故意不调用 apply_query_filter
        qb
    }
}
```

**从简分析**：
- DbSet 的 `query()` 在 [db_set.rs:248-257](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_set.rs#L248-L257) 中通过 `apply_query_filter` 应用过滤器
- `query_ignore_filters()` 复用 QueryBuilder 构造逻辑，**故意跳过** `apply_query_filter` 调用
- **无需**给 QueryBuilder 加 `ignore_filters` 标志位——过滤器是在 DbSet 层应用（构造时），不是在 QueryBuilder 层应用（执行时）
- 5 行实现，零侵入

**测试**（同文件或独立测试）：
```rust
// 设置过滤器后，query_ignore_filters().to_list() 返回全部行（含其他租户）
// query().to_list() 只返回当前租户行
```

---

### Task 3b：NavigationLoader 应用查询过滤器

**目标**：导航加载的 `SELECT * FROM related WHERE fk IN (...)` 追加过滤器，防止加载跨租户关联实体。

**改动文件**：
1. `crates/core/src/model_builder.rs`（新增批量收集方法）
2. `crates/core/src/db_set.rs`（存储 filter_map）
3. `crates/core/src/query.rs`（传递 filter_map）
4. `crates/core/src/navigation_loader.rs`（应用 filter）

**方案设计——`QueryFilterMap`**：

定义一个按表名查找过滤器的轻量映射，避免泛型扩散：

```rust
// model_builder.rs 新增
impl ModelBuilder {
    /// Collects all registered query filters keyed by table name.
    /// Used by NavigationLoader to apply tenant isolation to secondary queries.
    pub fn filters_by_table(&self) -> HashMap<String, BoolExpr> {
        let mut map = HashMap::new();
        for (type_id, config) in &self.configs {
            if let Some(filter) = &config.query_filter {
                // 从 entity_metas 或 config 中取 table_name
                if let Some(meta) = self.entity_metas.get(type_id) {
                    map.insert(meta.table_name.to_string(), filter.clone());
                }
            }
        }
        map
    }
}
```

> **从简**：遍历一次 configs，O(n) 构建 map。DbContext 在创建 DbSet 时构建一次，Arc 共享给所有 DbSet。

**改动 1 — DbContext 构建 filter_map 并存入 DbSet**（[db_context.rs:281-294](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L281-L294)）：

在 `set::<T>()` 创建 DbSet 时，附带传入全局 filter_map：
```rust
self.sets.entry(type_id).or_insert_with(|| {
    let table_name = /* ... */;
    let mut db_set = DbSet::<T>::with_provider(table_name, Arc::clone(&self.provider));
    if let Some(filter) = self.model_builder.get_query_filter(&type_id) {
        db_set.set_query_filter(filter.clone());
    }
    // 新增：传入全局 filter_map（供 NavigationLoader 使用）
    let filter_map = self.model_builder.filters_by_table();
    db_set.set_filter_map(Arc::new(filter_map));
    Box::new(db_set)
});
```

**改动 2 — DbSet 新增字段与 setter**（db_set.rs）：
```rust
pub struct DbSet<T: IEntityType> {
    pub(crate) entries: Vec<TrackedEntry<T>>,
    table_name: String,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    query_filter: Option<BoolExpr>,
    filter_map: Option<Arc<HashMap<String, BoolExpr>>>,  // 新增
}

impl<T: IEntityType + IEntitySnapshot> DbSet<T> {
    pub fn set_filter_map(&mut self, map: Arc<HashMap<String, BoolExpr>>) {
        self.filter_map = Some(map);
    }
    // ...
}
```

**改动 3 — QueryBuilder 携带 filter_map**（query.rs）：
```rust
pub struct QueryBuilder<T: IEntityType> {
    state: QueryState,
    provider: Option<Arc<dyn IDatabaseProvider>>,
    filter_map: Option<Arc<HashMap<String, BoolExpr>>>,  // 新增
    _phantom: PhantomData<T>,
}
```

`DbSet::query()` 和 `query_ignore_filters()` 传入 `self.filter_map.clone()`。

`load_includes` 调用点（[query.rs:1185](file:///d:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1185)）改为：
```rust
crate::navigation_loader::load_includes(
    &mut entities, &includes, &**provider, self.filter_map.as_deref()
).await?;
```

**改动 4 — NavigationLoader 应用 filter**（navigation_loader.rs）：

`load_includes` 签名增加 `filter_map: Option<&HashMap<String, BoolExpr>>`，透传到 `load_scalar_navigation` 和 `load_many_to_many`。

在 3 处裸 SQL 构造点（HasMany:117-122、BelongsTo/HasOne:150-155、ManyToMany related_sql:266-271）后追加：
```rust
// load_scalar_navigation HasMany 分支为例
let mut sql = format!(
    "SELECT * FROM {} WHERE {} IN ({})",
    related_table, fk_column, placeholders.join(", ")
);
let mut params: Vec<DbValue> = parent_ids;
if let Some(filter) = filter_map.and_then(|m| m.get(related_table)) {
    let mut idx = params.len() + 1;
    let filter_sql = compile_bool_expr(filter, gen, &mut idx);
    params.extend(collect_bool_expr_values(filter));
    sql = format!("{} AND ({})", sql, filter_sql);
}
let rows = conn.query(&sql, &params).await?;
```

> **从简**：复用已有的 `compile_bool_expr`/`collect_bool_expr_values`（已是 `pub(crate)`）；filter_map 查找 O(1)；参数追加在 IN 参数之后，索引正确。

**测试**（新建 `tests/query_filter_nav_tests.rs`）：
- 父实体（tenant_id=1）有 FK 指向子实体（tenant_id=2）
- 加载 navigation 后子实体为空（被过滤器拦截）

---

### Task 5：并发安全文档

**目标**：明确记录线程安全契约与多租户用法，避免用户误用。

**改动文件**：
1. `crates/core/src/db_context.rs` 模块文档顶部增加"线程安全"小节
2. `crates/core/src/di.rs` 模块文档增加"Scoped 生命周期"说明
3. 新建 `docs/rust-ef/03-advanced/multi-tenancy-foundation.md`

**db_context.rs 模块文档新增**（在第 1 行 `//!` 块内追加）：
```rust
//! ## 线程安全
//!
//! `DbContext` **非线程安全**——单个实例禁止跨线程共享。
//! 这是设计决策（对齐 EFCore），不是限制。
//!
//! **正确用法**：每个请求/操作创建一个 DI Scope：
//! ```rust,ignore
//! let scope = provider.create_dbcontext_scope();
//! let ctx = scope.get::<dyn IDbContext>().unwrap();
//! // 同一 scope 内多次 get 返回同一实例（单位工作）
//! ```
//!
//! **反模式**：`Arc<Mutex<DbContext>>` 共享会导致跟踪污染——
//! Thread A 的 `save_changes()` 会提交 Thread B 挂起的变更。
//!
//! 从根 `ServiceProvider` 直接 `get::<dyn IDbContext>()` 退化为每次新实例（等价 transient）。
```

**di.rs 模块文档新增**：
```rust
//! ## Scoped 生命周期
//!
//! `add_dbcontext` 注册为 `Scoped`——同一 `Scope` 内复用同一实例，
//! 不同 `Scope` 隔离。从根 `ServiceProvider` 直接解析退化为 `Transient`。
//!
//! 使用 `DbContextScopeExt::create_dbcontext_scope()` 创建作用域。
```

**新建 `docs/rust-ef/03-advanced/multi-tenancy-foundation.md`**：
- 线程安全契约（DbContext 非线程安全 + Scoped 隔离）
- 多租户：用 `has_query_filter` 注册租户隔离谓词
  - SELECT：自动应用
  - UPDATE/DELETE：自动应用（Task 3a）
  - Navigation：自动应用（Task 3b）
  - INSERT：用户在 `add()` 前手动设置 `tenant_id`（框架提供能力不写死）
- 跨租户管理查询用 `query_ignore_filters()`
- 反模式警示

---

## 四、假设与决策

| # | 决策 | 理由 |
|---|------|------|
| D1 | 不重复已完成工作（Task 1 Scoped、Task 2 双轨统一） | 探索已验证落盘；遵循"不重复已完成步骤" |
| D2 | `save_one_set` 签名不变（仍 `&mut DbSet<E>`），内部读取 filter | 避免 6 处调用点改动；`query_filter()` 是 `&self`，与 `tracked_by_state` 的 `&self` 兼容 |
| D3 | DbSet `query_filter()` getter 为 `pub(crate)` | 不暴露内部字段；仅 save_one_set 需要 |
| D4 | Task 3c 用"跳过 apply_query_filter"而非"标志位" | 过滤器在 DbSet 构造层应用，非 QueryBuilder 执行层；5 行实现 |
| D5 | Task 3b 用 `QueryFilterMap`（`HashMap<String, BoolExpr>`）而非 `QueryFilterResolver` trait | map 查找 O(1)，无需 trait object；DbContext 构建一次 Arc 共享 |
| D6 | `filters_by_table` 遍历 configs 一次构建 | O(n) 一次，Arc 共享给所有 DbSet；不额外遍历 |
| D7 | NavigationLoader filter_map 在 IN 参数后追加 | 索引连续；复用 compile_bool_expr 的 idx 机制 |
| D8 | Task 4（拦截器变更钩子）不在本计划范围 | 前序计划已标记可延后至 Phase 3；INSERT tenant_id 由用户手动设置满足"框架提供能力" |

---

## 五、验证步骤

### 5.1 逐 Task 验证

| Task | 测试文件 | 验证点 |
|------|----------|--------|
| 3a-finish | `tests/query_filter_exec_tests.rs` | UPDATE/DELETE 跨租户行被 WHERE 过滤（0 rows） |
| 3c | 同上或 `tests/ignore_query_filters_tests.rs` | `query_ignore_filters().to_list()` 返回全量行 |
| 3b | `tests/query_filter_nav_tests.rs` | 跨租户导航加载被过滤 |
| 5 | 文档检查 | 模块文档含线程安全小节 |

### 5.2 回归基线

- `cargo test --workspace -- --skip test_postgres_crud_lifecycle` 全过（postgres 环境性跳过）
- `cargo clippy --workspace --all-targets -- -D warnings` 清洁
- `cargo fmt --all --check` 清洁
- blog-example 运行成功

### 5.3 实施顺序

```
Task 3a-finish (DML 过滤器收尾) → cargo check + query_filter_exec_tests + 全量回归
Task 3c        (IgnoreFilters)   → 独立测试
Task 3b        (Nav 过滤器)       → query_filter_nav_tests + 全量回归
Task 5         (文档)             → 最后
```

每完成一个 Task 运行 `cargo test --workspace` 确保无回归。

---

## 六、文件变更清单

| 文件 | 操作 | Task |
|------|------|------|
| `crates/core/src/db_set.rs` | 新增 `query_filter()` getter + `filter_map` 字段 + `set_filter_map()` + `query_ignore_filters()` | 3a, 3b, 3c |
| `crates/core/src/db_context.rs` | `save_one_set` 透传 filter；`set::<T>()` 构建 filter_map 存入 DbSet | 3a, 3b |
| `crates/core/src/model_builder.rs` | 新增 `filters_by_table()` | 3b |
| `crates/core/src/query.rs` | `QueryBuilder` 加 `filter_map` 字段；`load_includes` 调用传入 | 3b |
| `crates/core/src/navigation_loader.rs` | `load_includes`/`load_scalar_navigation`/`load_many_to_many` 加 filter_map 参数 + 3 处 SQL 追加 | 3b |
| `crates/core/tests/query_filter_exec_tests.rs` | 新建 | 3a |
| `crates/core/tests/ignore_query_filters_tests.rs` | 新建（或合并到 3a 测试文件） | 3c |
| `crates/core/tests/query_filter_nav_tests.rs` | 新建 | 3b |
| `docs/rust-ef/03-advanced/multi-tenancy-foundation.md` | 新建 | 5 |
| `crates/core/src/db_context.rs` 模块文档 | 增加"线程安全"小节 | 5 |
| `crates/core/src/di.rs` 模块文档 | 增加"Scoped 生命周期"说明 | 5 |
