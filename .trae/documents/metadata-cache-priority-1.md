# 优先级 1：元数据进程级共享（Metadata Cache）

## Context（为什么做这个改动）

**问题**：当前每次 `DbContext::from_options()`（HTTP 请求级别通过 `get_owned()` 触发）都重新执行 `discover_entities()`：
1. 迭代 `inventory::iter::<EntityConfigRegistration>` 并调用 `apply_fn`（执行用户 `IEntityTypeConfiguration::configure()` 代码）
2. 迭代 `inventory::iter::<EntityRegistration>` 构造 `EntityTypeMeta`

结果对所有相同 `context_key` 的 `DbContext` 实例完全相同 —— 每请求重复解析是浪费。用户在深度分析中明确要求"实体、关系、设置元数据只解析一次，之后单例共享"。

**关键约束**：`ctx.model()` 返回 `&mut ModelBuilder`（[db_context.rs:401](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L401)），用户在 `from_options()` 之后通过 `ctx.model().has_query_filter(...)` 修改模型（见 [multi-tenancy-foundation.md](file:///e:/GitCode/RF/rust-ef/docs/rust-ef/03-advanced/multi-tenancy-foundation.md)）。**纯单例共享 ModelBuilder 不可行** —— 每个 DbContext 需要自己的 ModelBuilder 承载 per-instance 查询过滤器。

**预期结果**：`inventory::iter` + `configure()` 每个 `context_key` 只运行一次（进程级），结果通过 `Arc` 共享；每个 DbContext 仍保留可变的 `ModelBuilder` 用于用户后置修改。

---

## 设计方案

**在 `DbContextOptions` 上缓存 `discover_entities()` 的输出**。`DbContextOptions` 已通过 `Arc::new(builder.build())` 在 `add_dbcontext` 注册时被 `Arc` 共享（[di.rs:156,172](file:///e:/GitCode/RF/rust-ef/crates/core/src/di.rs#L156)），所有 `get_owned()` 调用复用同一 `Arc<DbContextOptions>` —— 缓存字段天然被共享。

### 核心策略
- **共享**：`BuiltMetadata`（`discover_entities()` 的输出）通过 `Arc<BuiltMetadata>` 共享
- **克隆**：`DbContext.entity_metas`（HashMap）和 `ModelBuilder.entity_metas`/`configs` 从缓存克隆 —— 保留 per-instance 可变性
- **不共享**：`EntityTypeMeta.property_index`/`navigation_index` 的 `OnceLock` 缓存（每 DbContext 重建，v1 接受此开销，~10-20 次 HashMap 插入/实体，相对 DB I/O 可忽略）

### v1 不做的事
- ❌ `Arc<HashMap>` 化 `entity_metas` —— 会破坏 `set::<T>()` 的 `entry()` API（[db_context.rs:372](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L372)）
- ❌ `Arc<EntityTypeMeta>` 化 —— API 变更大，收益小，留待 v2
- ❌ 独立 DI 服务承载缓存 —— `DbContextOptions` 已是天然共享点，无需额外注册

---

## 实现步骤

### 步骤 1：新建 `crates/core/src/metadata_cache.rs`

定义两个 `pub(crate)` 类型：

```rust
use crate::metadata::EntityTypeMeta;
use crate::model_builder::EntityConfig;  // 需改为 pub(crate)
use crate::registration::{EntityConfigRegistration, EntityRegistration};
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct BuiltMetadata {
    pub entity_metas: HashMap<TypeId, EntityTypeMeta>,
    pub model_metas: Vec<EntityTypeMeta>,
    pub configs: HashMap<TypeId, EntityConfig>,
}

pub(crate) struct MetadataCache {
    by_key: Mutex<HashMap<Option<String>, Arc<BuiltMetadata>>>,
}

impl MetadataCache {
    pub fn new() -> Self { ... }
    
    pub fn get_or_build(&self, context_key: Option<&str>) -> Arc<BuiltMetadata> {
        // 1. lock + lookup by context_key.map(String::from)
        // 2. hit → Arc::clone
        // 3. miss → Self::build(context_key) → Arc::new → insert → return
    }
    
    fn build(context_key: Option<&str>) -> BuiltMetadata {
        // 把 db_context.rs:432-457 discover_entities() 的 inventory::iter 逻辑搬来：
        // 1. 创建临时 ModelBuilder，迭代 EntityConfigRegistration 调用 apply_fn
        // 2. 迭代 EntityRegistration 构造 EntityTypeMeta
        // 3. 从临时 ModelBuilder 提取 configs（需 ModelBuilder 暴露 pub(crate) 接口）
        // 4. 返回 BuiltMetadata { entity_metas, model_metas, configs }
    }
}
```

### 步骤 2：修改 `crates/core/src/model_builder.rs`

1. 把 `EntityConfig`（[L23](file:///e:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L23)）和 `PropertyConfigOverride`（[L32](file:///e:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L32)）从私有改为 `pub(crate)`
2. 新增 `pub(crate) fn from_built(built: &BuiltMetadata) -> Self` 构造器：
   ```rust
   pub(crate) fn from_built(built: &BuiltMetadata) -> Self {
       Self {
           entity_metas: built.model_metas.clone(),
           configs: built.configs.clone(),
           build_cache: OnceLock::new(),
           filter_cache: OnceLock::new(),
       }
   }
   ```
3. 新增 `pub(crate) fn configs(&self) -> &HashMap<TypeId, EntityConfig>` 访问器（供 `MetadataCache::build` 提取）
4. 新增 `pub(crate) fn entity_metas_vec(&self) -> &[EntityTypeMeta]` 访问器（同上）

### 步骤 3：修改 `crates/core/src/db_context.rs`

1. **`DbContextOptions` 新增字段**（[L79-94](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L79)）：
   ```rust
   pub(crate) metadata_cache: Arc<MetadataCache>,
   ```
2. **`DbContextOptionsBuilder::new()` 初始化**：`metadata_cache: Arc::new(MetadataCache::new())`
3. **`DbContextOptions::default()` 同步初始化**（Default impl 在 [L129-140] 附近）—— 必须加 `metadata_cache: Arc::new(MetadataCache::new())`，否则编译错误
4. **重写 `from_options()`**（[L337-355](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L337)）：
   ```rust
   pub fn from_options(options: &DbContextOptions) -> EFResult<Self> {
       let provider = options.create_provider()?;
       let built = options.metadata_cache.get_or_build(options.context_key.as_deref());
       let ctx = Self {
           sets: HashMap::new(),
           savers: HashMap::new(),
           entity_metas: built.entity_metas.clone(),
           model_builder: ModelBuilder::from_built(&built),
           change_tracker: ChangeTracker::new(),
           provider,
           interceptor_pipeline: InterceptorPipeline::new(options.interceptors.clone()),
           lazy_loading_enabled: options.lazy_loading_enabled,
           context_key: options.context_key.clone(),
       };
       Ok(ctx)
   }
   ```
5. **`discover_entities()` 改为 no-op**（[L432-457](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L432)）：保留方法签名（向后兼容），body 改为 `Ok(())`，更新文档说明"元数据已在 `from_options()` 中从缓存加载，此方法为 no-op"
6. 在文件顶部 `mod metadata_cache;` + `use metadata_cache::{BuiltMetadata, MetadataCache};`

### 步骤 4：修改 `crates/core/src/lib.rs`

添加 `mod metadata_cache;`（或 `pub(crate) mod metadata_cache;`）

---

## 关键设计决策（来自 Plan agent 验证）

| 决策 | 选择 | 理由 |
|---|---|---|
| 缓存放哪 | `DbContextOptions` 字段 | 已 `Arc` 共享，无需额外 DI 注册 |
| `entity_metas` 共享方式 | 克隆（非 Arc） | 保留 `set::<T>()` 的 `entry()` API |
| `ModelBuilder` 共享方式 | 克隆 configs + entity_metas | 保留 `has_query_filter()` per-instance 修改 |
| `EntityTypeMeta` 共享方式 | 不共享（v1） | OnceLock 重建开销可忽略；Arc 化留待 v2 |
| `MetadataCache` 锁粒度 | `Mutex<HashMap>` 仅 `get_or_build` 持锁 | 标准 get-or-insert，返回 `Arc` 后无锁 |
| 新文件 vs 内联 | 新文件 `metadata_cache.rs` | 匹配现有 `model_builder.rs`/`metadata.rs` 布局，`db_context.rs` 已 748 行 |

---

## 验证方案

### 编译验证
```powershell
cargo build -p rust-ef
cargo build --workspace
```

### 单元测试验证
```powershell
# 1. 上下文隔离（cache 按 context_key 隔离）
cargo test -p rust-ef --test multi_db_context_tests

# 2. ModelBuilder 缓存逻辑（per-instance 保留）
cargo test -p rust-ef --test model_builder_cache_tests

# 3. 自动注册 + 向后兼容
cargo test -p rust-ef --test auto_registration_tests
cargo test -p rust-ef --test auto_registration_integration_tests

# 4. Scoped/Owned 生命周期
cargo test -p rust-ef --test scoped_lifecycle_tests
cargo test -p rust-ef --test owned_injection_tests

# 5. 事务原子性（不应受影响）
cargo test -p rust-ef --test transaction_composite_tests

# 6. 跟踪一致性
cargo test -p rust-ef --test tracking_consistency_tests

# 7. 种子数据（has_data 配置走 cache.configs）
cargo test -p rust-ef --test sqlite_crud_tests
cargo test -p rust-ef --test integration_tests
```

### 新增测试（在 `crates/core/tests/metadata_cache_tests.rs`）

验证三点：
1. **共享性**：同一 `DbContextOptions` 创建两个 `DbContext`，断言 `ctx1.entity_metas_contains::<Blog>()` 与 `ctx2` 相同（已有 API 不足以验证内部共享，需用 `Arc::ptr_eq` 或新增测试钩子）
2. **隔离性**：`add_dbcontext_keyed("primary")` + `add_dbcontext_keyed("logs")`，断言 primary 只含 Blog、logs 只含 LogEntry（已有 `multi_db_context_tests` 覆盖，可复用）
3. **per-instance 修改保留**：`ctx1.model().has_query_filter::<Blog>(...)` 不影响 `ctx2` 的 `model_builder.get_query_filter(Blog)`

### 性能验证（可选）
对 `from_options()` 做 1000 次循环测时，对比改动前后。预期首调用后 ~90% 时间下降（inventory 迭代 + configure 只跑一次）。

---

## 关键文件清单

| 文件 | 改动类型 |
|---|---|
| `crates/core/src/metadata_cache.rs` | **新建**：`BuiltMetadata` + `MetadataCache` |
| `crates/core/src/lib.rs` | 添加 `mod metadata_cache;` |
| `crates/core/src/model_builder.rs` | `EntityConfig`/`PropertyConfigOverride` 改 `pub(crate)`；新增 `from_built()` + 2 个 `pub(crate)` 访问器 |
| `crates/core/src/db_context.rs` | `DbContextOptions` 加字段；`from_options` 重写；`discover_entities` 改 no-op；`Default` 同步 |
| `crates/core/tests/metadata_cache_tests.rs` | **新建**：3 个新测试 |
| `CHANGELOG.md` | 新增 `[Unreleased]` 条目 |

---

## 不做的事（CLAUDE.md "Simplicity First"）

- ❌ 不重构 `EntityTypeMeta` 为 `Arc` 内部结构
- ❌ 不引入 `IModel` trait 抽象（EFCore 风格的 `IModel`/`IEntityType` 层级）—— 当前 `BuiltMetadata` 足够
- ❌ 不改 `discover_entities()` 公开签名（向后兼容）
- ❌ 不动 `set::<T>()`、`save_changes()`、`model()` 等公开 API
- ❌ 不删除现有 `OnceLock` 缓存（`build_cache`/`filter_cache` 仍是 per-instance，与 cache 层各司其职）
- ❌ 不更新文档示例（API 未变，文档无需改）
