# Phase 3：多租户性能优化与缓存策略

> **状态**：待审批
> **前置**：Phase 2 多租户 DML 隔离收尾已完成（SELECT/UPDATE/DELETE/Navigation 全覆盖）
> **原则**：充分考虑 REF 框架性能前提下，从简实现；框架提供能力不写死

---

## 1. 执行摘要

Phase 3 聚焦**现有架构的性能优化**，不引入 Identity Map / Lazy Loading 等架构性变更（推迟至 v0.7+/v0.8+）。通过**缓存层、批量化、零分配**三个维度，将多租户查询路径的 round trips 从 O(N) 降至 O(1)，消除热路径堆分配，使元数据访问从 O(N) 线性扫描降至 O(1) 哈希查找。

**核心收益预期**：

| 指标 | 当前 | Phase 3 后 | 改善 |
|------|------|-----------|------|
| N 行 INSERT round trips | N | 1 | O(N) → O(1) |
| N 行 DELETE round trips | N | 1 | O(N) → O(1) |
| `model_builder.build()` 调用 | 每次全量重建 | OnceCell 缓存 | O(N) → O(1) |
| `sql_generator()` 分配 | 每次 Box | 返回引用 | 堆分配 → 零分配 |
| metadata `find_navigation` | O(N) 线性 | O(1) HashMap | 线性 → 常数 |
| filter SQL 编译 | 每查询一次 | 预编译缓存 | O(N) → O(1) |

---

## 2. 当前架构性能瓶颈（审计结论）

### P0 — 严重瓶颈

| # | 位置 | 问题 | 影响 |
|---|------|------|------|
| P0-1 | `change_executor.rs:35,93,178` | INSERT/UPDATE/DELETE 逐行 round trip（N+1 DML） | N 行 = N 次网络往返 |
| P0-2 | `sqlite/provider.rs:10` | 单 Mutex 连接，全访问串行化 | 并发读写完全串行 |

### P1 — 明显瓶颈

| # | 位置 | 问题 | 影响 |
|---|------|------|------|
| P1-1 | `model_builder.rs:84` | `build()` 零缓存，每次全量深克隆 | O(N) 克隆 × O(N) 调用 = O(N²) |
| P1-2 | `db_context.rs:304,314` | `set::<T>()` 冷路径触发 `build()` + `filters_by_table()` 全量重建 | 注册 N 实体 = O(N²) |
| P1-3 | `change_executor.rs` | 每行重新生成 SQL 字符串（同表同列模板相同） | O(N) 次 format! |

### P2 — 可优化项

| # | 位置 | 问题 | 影响 |
|---|------|------|------|
| P2-1 | `provider.rs:355` | `sql_generator()` 返回 `Box<dyn>` | 每次堆分配 |
| P2-2 | `navigation_loader.rs:278-284` | `related_keys` 去重用线性查找 | O(K²) |
| P2-3 | `navigation_loader.rs:336` | `db_value_key` 用 `format!` 做 HashMap 键 | O(rows) 次字符串分配 |
| P2-4 | `navigation_loader.rs:12-25` | `apply_filter_to_sql` 每查询重新编译 filter | 重复编译 |
| P2-5 | `metadata.rs:209-221` | `find_navigation`/`find_property` 线性扫描 | O(n) per call |

### P3 — 配置缺陷

| # | 位置 | 问题 |
|---|------|------|
| P3-1 | `postgres/di_extension.rs:15` | 池大小硬编码 5，不可配置 |
| P3-2 | `mysql/provider.rs:6` | 未暴露 `PoolOptions`（max_connections 等） |
| P3-3 | `sqlite/connection.rs:7` | 无 WAL 模式，读写互斥 |

---

## 3. 设计决策

### D1：ModelBuilder 用 OnceCell 缓存 build() 结果

`ModelBuilder` 在 `discover_entities()` / 配置阶段后内容稳定。`build()` 结果可缓存为 `OnceCell<Vec<EntityTypeMeta>>`，`filters_by_table()` 结果缓存为 `OnceCell<Arc<HashMap<String, BoolExpr>>>`。

- **失效策略**：任何 `register_entity_meta()` / `has_query_filter()` / Fluent API 配置调用时调用 `OnceCell::take()` 清除缓存
- **复杂度**：低（加 2 个字段 + 3 处 take）
- **收益**：O(N) → O(1)，消除 O(N²) 注册开销

### D2：批量 INSERT — 多值 VALUES 语法

将 N 行 INSERT 合并为单条多值 INSERT：
```sql
INSERT INTO blogs (url, tenant_id) VALUES (?, ?), (?, ?), (?, ?)
```

- **兼容性**：SQLite ≥3.7.11、MySQL、PostgreSQL 均支持
- **参数限制**：SQLite 默认 999 参数上限。当 `行数 × 列数 > 900` 时自动分批
- **自增主键回填**：批量 INSERT 后用 `last_insert_rowid()` + 递增序号回填（SQLite）；MySQL 用 `LAST_INSERT_ID()` + 批量查；PostgreSQL 用 `RETURNING`
- **复杂度**：中（需处理分批 + 主键回填）
- **收益**：O(N) round trips → O(N/批量大小) round trips，通常 1 次

### D3：批量 DELETE — WHERE pk IN (...) 语法

将 N 行 DELETE 合并为：
```sql
DELETE FROM blogs WHERE id IN (?, ?, ?) AND tenant_id = ?
```

- **条件**：仅当所有 DELETE 行属于同一表且无自定义 concurrency token 时合并
- **复杂度**：低（收集 PK → 拼 IN 子句 → 1 次 execute）
- **收益**：O(N) → O(1) round trips

### D4：UPDATE 暂不批量，但加 SQL 模板缓存

CASE WHEN 批量 UPDATE 语法复杂、各方言不同、且多租户场景下每行 SET 列可能不同（只有变更列才更新）。Phase 3 保留逐行 UPDATE，但按 `(table, set_columns)` 缓存 SQL 模板，避免每行重新 `gen.update()`。

- **缓存位置**：`ChangeExecutor` 内 `HashMap<(String, Vec<String>), String>`
- **失效**：DbContext 生命周期内不变（DbContext 是 Scoped，每请求一个实例）
- **复杂度**：低
- **收益**：消除 O(N) 次 format!

### D5：sql_generator() 改为返回引用

`ISqlGenerator` 各实现均无状态，`sql_generator()` 可改为 `&dyn ISqlGenerator` 或直接 `&'static dyn ISqlGenerator`。

- **方案**：各 Provider 持有 `static` 实例，`sql_generator()` 返回 `&'static dyn ISqlGenerator`
- **影响面**：`IDatabaseProvider` trait + 3 个 provider 实现 + 所有调用点（`navigation_loader.rs`、`change_executor.rs`、`query.rs`）
- **复杂度**：中（机械性改动，无逻辑变更）
- **收益**：消除热路径每次堆分配

### D6：Metadata 用 HashMap 索引替代 Vec 线性扫描

`EntityTypeMeta` 内 `properties: Vec<PropertyMeta>` 和 `navigations: Vec<NavigationMeta>` 增加 `HashMap<String, usize>` 索引（属性名 → 下标），在 `build()` 时一次性构建。

- `find_navigation(name)` / `find_property(name)`：O(1)
- **复杂度**：低（加索引字段 + build 时构建）
- **收益**：navigation_loader / change_executor 中每实体查找从 O(n) → O(1)

### D7：连接池可配置 + SQLite WAL

- **PostgreSQL**：`use_postgres(cs, pool_size)` 暴露 `pool_size` 参数
- **MySQL**：`use_mysql(cs, |o| o.max_connections(20))` 暴露 `sqlx::PoolOptions`
- **SQLite**：开启 WAL 模式 + `busy_timeout`，允许读写并发
- **复杂度**：低
- **收益**：PostgreSQL/MySQL 可调优；SQLite 读不阻塞写

### D8：多租户过滤器 SQL 预编译缓存

`filter_map` 中的 `BoolExpr` 在 DbContext 生命周期内不变。可在 `filters_by_table()` 构建时预编译每个 filter 的 SQL 片段和参数值，缓存为 `Arc<HashMap<String, (String, Vec<DbValue>)>>`。

```rust
pub struct CompiledFilter {
    pub sql_fragment: String,     // e.g. "tenant_id = ?"
    pub params: Vec<DbValue>,     // e.g. [I32(1)]
}
```

- `apply_filter_to_sql` 改为直接拼接预编译片段，跳过 `compile_bool_expr` + `collect_bool_expr_values`
- **复杂度**：低（预编译一次，复用 N 次）
- **收益**：消除 navigation_loader 每查询的 filter 编译开销

---

## 4. 实施计划

### Task 1：ModelBuilder 缓存层（P1-1, P1-2）

**目标**：`build()` 和 `filters_by_table()` 结果缓存，消除 O(N²) 注册开销

**文件变更**：

| 文件 | 变更 |
|------|------|
| `model_builder.rs` | 加 `build_cache: OnceCell<Vec<EntityTypeMeta>>` + `filter_cache: OnceCell<Arc<HashMap<String, BoolExpr>>>`；`build()`/`filters_by_table()` 改为先查缓存；`register_entity_meta()`/`has_query_filter()`/`entity().to_table()` 等变更方法调 `take()` 失效 |

**验证**：
- 现有 65+ 测试全过
- 新增 `model_builder_cache_tests.rs`：验证首次 build 后缓存命中、变更后失效重建

### Task 2：批量 INSERT + 批量 DELETE（P0-1）

**目标**：N 行 INSERT/DELETE 从 N 次 round trip 降至 1 次（或分批）

**文件变更**：

| 文件 | 变更 |
|------|------|
| `change_executor.rs` | `execute_inserts` 改为批量多值 INSERT（自动分批 ≤900 参数）；`execute_deletes` 改为 `DELETE WHERE pk IN (...)`；保留逐行 fallback（有 concurrency token 时） |
| `provider.rs` | `ISqlGenerator` 加 `insert_batch(table, columns, row_count) -> String` 和 `delete_by_pks(table, pk_column, count) -> String` |
| `sqlite_sql_generator.rs` | 实现批量 SQL 生成 |
| `postgres_sql_generator.rs` | 实现批量 SQL 生成 |
| `mysql_sql_generator.rs` | 实现批量 SQL 生成 |

**验证**：
- 现有 DML 测试全过
- 新增 `batch_dml_tests.rs`：验证 N 行 INSERT 1 次 round trip、N 行 DELETE 1 次 round trip、自增主键回填正确、参数超限自动分批

### Task 3：SQL 模板缓存 + sql_generator() 零分配（P1-3, P2-1）

**目标**：消除热路径堆分配 + 避免重复 SQL 生成

**文件变更**：

| 文件 | 变更 |
|------|------|
| `provider.rs` | `sql_generator()` 返回 `&'static dyn ISqlGenerator`；trait 签名变更 |
| `sqlite/provider.rs` | 持有 `static SqliteSqlGenerator`，返回引用 |
| `postgres/provider.rs` | 持有 `static PostgresSqlGenerator`，返回引用 |
| `mysql/provider.rs` | 持有 `static MySqlSqlGenerator`，返回引用 |
| `change_executor.rs` | 加 `sql_templates: HashMap<(String, Vec<String>), String>` 缓存 UPDATE SQL 模板 |
| `navigation_loader.rs` | 调用点 `&*gen` → `gen`（已是引用） |
| `query.rs` | 调用点适配 |

**验证**：
- clippy 零警告
- 现有测试全过

### Task 4：NavigationLoader + Metadata 优化（P2-2 ~ P2-5, P2-4, D6, D8）

**目标**：消除行克隆、O(N²) 去重、filter 重复编译；metadata O(1) 查找

**文件变更**：

| 文件 | 变更 |
|------|------|
| `metadata.rs` | `EntityTypeMeta` 加 `property_index: HashMap<String, usize>` + `navigation_index: HashMap<String, usize>`；`build()` 时构建；`find_*` 改用索引 |
| `model_builder.rs` | `filters_by_table()` 返回 `Arc<HashMap<String, CompiledFilter>>`；`CompiledFilter { sql_fragment, params }` |
| `navigation_loader.rs` | `apply_filter_to_sql` 改用 `CompiledFilter` 直接拼接；`related_keys` 去重改用 `HashSet`；`db_value_key` 优化 |
| `query.rs` | `QueryBuilder.filter_map` 类型从 `Arc<HashMap<String, BoolExpr>>` 改为 `Arc<HashMap<String, CompiledFilter>>` |
| `db_set.rs` | `filter_map` 类型同步更新 |

**验证**：
- 现有 navigation + filter 测试全过
- 新增 `navigation_perf_tests.rs`：验证大结果集（1000 行）导航加载无 O(N²) 去重

### Task 5：连接池可配置 + SQLite WAL（P0-2, P3-1 ~ P3-3）

**目标**：连接池大小可配置；SQLite WAL 模式支持读写并发

**文件变更**：

| 文件 | 变更 |
|------|------|
| `postgres/di_extension.rs` | `use_postgres(cs, pool_size)` 暴露 pool_size |
| `mysql/di_extension.rs` | `use_mysql(cs, configure)` 暴露 `FnOnce(&mut PoolOptions)` |
| `sqlite/provider.rs` | `use_sqlite(cs, pragmas)` 或默认开启 `PRAGMA journal_mode=WAL` + `PRAGMA busy_timeout=5000` |
| `sqlite/connection.rs` | 支持多连接（从文件路径打开多个 Connection，而非共享 Mutex） |

**验证**：
- 现有 provider 测试全过
- 新增 `connection_pool_tests.rs`：验证 PostgreSQL pool_size 可配、SQLite WAL 生效

---

## 5. 实施顺序与依赖

```
Task 1 (ModelBuilder 缓存)     ← 无依赖，可独立实施
    ↓
Task 4 (NavigationLoader 优化)  ← 依赖 Task 1 的 CompiledFilter
    ↓
Task 2 (批量 DML)              ← 独立，但建议在 Task 3 后
    ↓
Task 3 (SQL 模板缓存 + 零分配)  ← 独立
    ↓
Task 5 (连接池)                ← 独立
```

**推荐顺序**：Task 1 → Task 3 → Task 2 → Task 4 → Task 5

理由：Task 1 是其他缓存的基础；Task 3 的 `sql_generator()` 改动会影响 Task 2 的接口设计；Task 4 依赖 Task 1 的 `CompiledFilter`；Task 5 完全独立可并行。

---

## 6. 多租户缓存策略总结

### 缓存层次

```
┌──────────────────────────────────────────────────────┐
│ DbContext (Scoped, 每请求一个实例)                     │
│                                                       │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────┐ │
│  │ ModelBuilder │  │ ChangeExec   │  │ DbSet        │ │
│  │ OnceCell    │  │ SQL 模板缓存  │  │ filter_map   │ │
│  │ - build()   │  │ HashMap<      │  │ Arc<HashMap> │ │
│  │ - filters   │  │  (table,cols) │  │ CompiledFilter│ │
│  └─────────────┘  └──────────────┘  └──────────────┘ │
│                                                       │
│  ┌─────────────────────────────────────────────────┐  │
│  │ Provider (Singleton, 跨请求共享)                  │  │
│  │  - &'static ISqlGenerator (零分配)              │  │
│  │  - Connection Pool (PostgreSQL/MySQL)            │  │
│  └─────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────┘
```

### 缓存失效策略

| 缓存 | 生命周期 | 失效条件 |
|------|----------|----------|
| `build()` 结果 | Scoped (DbContext) | 任何 `register_entity_meta` / `has_query_filter` / Fluent API 调用 |
| `filters_by_table()` | Scoped (DbContext) | 同上（共享失效） |
| `CompiledFilter` | Scoped (DbContext) | 同上 |
| SQL 模板缓存 | Scoped (DbContext) | DbContext 释放时自动清除 |
| `&'static ISqlGenerator` | 进程级 | 永不失效（无状态） |
| Metadata 索引 | Scoped (DbContext) | `build()` 时构建，随 `build()` 缓存 |

### 不缓存的项

| 项 | 原因 |
|----|------|
| 查询结果 | 多租户场景下缓存键需包含租户 ID + 查询条件，失效复杂度高，收益不确定 |
| 实体实例 (Identity Map) | 属 G6 架构性变更，推迟至 v0.7.5 |
| 连接实例 | 由 provider 连接池管理，框架不介入 |

---

## 7. 不在 Phase 3 范围内

| 项 | 原因 | 归属 |
|----|------|------|
| Identity Map | 架构性变更，需 `EntityRef<T>` 句柄 + `Weak<RefCell>` 自引用 | v0.7.5 (G6) |
| Lazy Loading | 需 `LazyBelongsTo<T>` + `RwLock` + N+1 检测 | v0.8.0 (G7) |
| 子查询 / 关联过滤 | `BoolExpr::Exists` + 表别名机制 | v0.7.0 (G5) |
| 导航 Fixup | 双向自动填充导航属性 | v0.8.5 (G8) |
| 批量 UPDATE (CASE WHEN) | 语法复杂、方言差异大、收益不如 INSERT/DELETE 显著 | v1.0 收尾 |
| 查询结果缓存 | 多租户缓存失效复杂度高 | 暂不实施 |

---

## 8. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| 批量 INSERT 自增主键回填不一致 | 不同数据库的自增行为不同 | SQLite: `last_insert_rowid()` + 递增序号；MySQL: `LAST_INSERT_ID()` + 查询；PostgreSQL: `RETURNING` 子句 |
| OnceCell 缓存失效遗漏 | 配置变更后使用过期缓存 | 所有变更方法统一调用 `invalidate_cache()` 内部方法；加 `#[cfg(test)]` 断言缓存被清除 |
| `sql_generator()` 签名变更影响面大 | 3 个 provider + 多个调用点 | 机械性改动，编译器全量检查；先改 trait 再逐个 provider 适配 |
| SQLite WAL 模式行为变化 | WAL 模式下文件行为不同（-wal/-shm 文件） | 文档说明；保留回退选项（通过 pragma 禁用） |
| 批量 DELETE 丢失行级 concurrency token | 合并后无法逐行检查 token | 仅有 concurrency token 的行退回逐行模式；无 token 的行批量 |

---

## 9. 验证步骤

1. `cargo check --workspace` — 编译通过
2. `cargo clippy --workspace --tests` — 零警告
3. `cargo fmt --all -- --check` — 格式清洁
4. `cargo test -p rust-ef -- --skip postgres` — 全量测试通过（postgres 环境性跳过）
5. 新增测试文件全过：
   - `model_builder_cache_tests.rs`
   - `batch_dml_tests.rs`
   - `navigation_perf_tests.rs`
   - `connection_pool_tests.rs`
6. `examples/blog-example` 运行成功
7. 性能基准对比（可选）：用 `criterion` 对比 Phase 2 vs Phase 3 的批量 DML 吞吐
