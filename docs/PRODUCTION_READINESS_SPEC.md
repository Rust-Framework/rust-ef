# rust-ef 生产就绪技术规格说明书

> 版本: v0.5 — 基于 2026-06-26 审计结果  
> 包名: `rust-ef`（workspace: `crates/core`）  
> 目标: 逐步推进至 v1.0 生产就绪状态  
> **当前阶段: RC 1 接近完成（约 98% 就绪度，P0 blocker 已全部清除）**

---

## 执行摘要

rust-ef v0.5 已具备 EF Core 风格 ORM 的**核心骨架**：类型映射式 `DbContext`、通用 `save_changes()`、`linq!` 查询 DSL、导航 Include、M2M、迁移引擎库 API + CLI 工具、DI 集成、子查询/关联过滤、乐观并发、全局查询过滤器、SaveChanges 拦截器、chrono/uuid/decimal 可选类型支持、exists_by_id/exists_by_key 存在性检查、事务回滚与复合主键 CRUD 集成测试。在 **SQLite / PostgreSQL / MySQL** 上有完整的 CRUD 集成测试（208 个测试全绿），CI 三库 matrix 已就位。

**已具备通用生产条件**，剩余缺口仅为 P1 polish 项（Lazy Loading、Provider 原生类型绑定等）。

| 场景 | 建议 |
|------|------|
| SQLite 原型 / 内部工具 | ✅ 可用 |
| PostgreSQL / MySQL 生产 | ✅ 可用（需自行集成测试验证） |
| 多写并发 + 乐观锁 | ✅ 可用（token 冲突检测） |
| 团队迁移 CLI 工作流 | ✅ 可用（add/apply/revert/list/script） |

---

## 里程碑总览

```
Alpha 2 (35%) ──► v0.3.5 (~60%) ──► Beta 1 (~85%) ──► 当前 v0.5 (~98%) ──► 1.0
                                                    ↑
                                            RC1 核心项已完成
                                            P0 已清除，剩余 P1 polish
```

---

## 当前已实现能力清单

### 架构（v0.3+）

| 能力 | 状态 | 位置 |
|------|:----:|------|
| 类型映射 `DbContext`（`ctx.set::<T>()`） | ✅ | `crates/core/src/db_context.rs` |
| `Arc<dyn IDbContext>` DI | ✅ | `crates/core/src/di.rs` |
| Keyed 多库注册 | ✅ | `add_dbcontext_keyed` |
| Provider 工厂注入 | ✅ | `DbContextOptions::provider_factory` |
| `SetOps<T>` 类型擦除 SaveChanges | ✅ | `db_context.rs` |
| SaveChanges 拦截器 | ✅ | `crates/core/src/interceptor.rs` |

### 实体与持久化

| 能力 | 状态 | 说明 |
|------|:----:|------|
| `#[derive(EntityType)]` | ✅ | 12+ 属性 |
| `IGetKeyValues` / `IEntitySnapshot` | ✅ | derive 自动生成 |
| `ChangeExecutor` INSERT/UPDATE/DELETE | ✅ | 参数化 + 事务 |
| INSERT 主键回填（RETURNING） | ✅ | SQLite 已测 |
| `ensure_created` / `ensure_deleted` | ✅ | 集成测试覆盖 |
| `remove_range` / `load_all` | ✅ | `db_set.rs` |
| 乐观并发 `#[concurrency_check]` | ✅ | UPDATE/DELETE WHERE 含 token |
| chrono / uuid / decimal 类型支持 | ✅ | 可选 feature（`chrono` / `uuid` / `decimal`） |

### 查询

| 能力 | 状态 | 说明 |
|------|:----:|------|
| `BoolExpr`（AND/OR/NOT/Raw） | ✅ | `query.rs` |
| `linq!` 表达式树 | ✅ | `crates/macros/src/linq.rs` |
| IN / BETWEEN / IS NULL / contains | ✅ | linq + QueryBuilder |
| join / group_by / having / 分页 | ✅ | QueryBuilder |
| 全局 Query Filter 自动注入 | ✅ | `ModelBuilder::has_query_filter` |
| 子查询 / 关联过滤 | ✅ | `any`/`none`/`all` → EXISTS/NOT EXISTS |
| `query_ignore_filters()` | ✅ | 管理员查询绕过过滤器 |

### 关系

| 能力 | 状态 | 说明 |
|------|:----:|------|
| BelongsTo / HasMany / HasOne | ✅ | |
| Many-to-Many（Join 实体） | ✅ | `m2m_tests.rs` |
| Include / ThenInclude 物化 | ✅ | `navigation_loader.rs` |
| Lazy Loading | ❌ | 未实现 |

### 迁移

| 能力 | 状态 | 说明 |
|------|:----:|------|
| 模型快照 diff | ✅ | 建/删表、增/删/改列 |
| Up/Down SQL 三方言 | ✅ | PG / MySQL / SQLite |
| `MigrationEngine::apply()` | ✅ | 单迁移执行 |
| `__ef_migrations_history` 表 DDL | ✅ | |
| 读取 history 跳过已应用迁移 | ✅ | `apply_pending` / `is_applied` |
| Revert 工作流 | ✅ | `revert` / `revert_last` / `revert_to_target` |
| FK/索引 diff | ✅ | `AddForeignKey` / `DropForeignKey` / `CreateIndex` / `DropIndex` |
| 迁移脚本生成 | ✅ | `generate_script(from, to)` — 前向/反向 SQL |
| 读取 history 跳过已应用 | ✅ | `get_applied_migrations` / `apply_pending` |

### Provider

| Provider | 实现 | 集成测试 |
|----------|:----:|:--------:|
| SQLite (`rust-ef-sqlite`) | ✅ | ✅ 9 CRUD + 导航/M2M |
| PostgreSQL (`rust-ef-postgres`) | ✅ | ✅ 可选（`RUST_EF_PG_URL` / CI） |
| MySQL (`rust-ef-mysql`) | ✅ | ✅ 可选（`RUST_EF_MYSQL_URL` / CI） |

### 工程化

| 能力 | 状态 |
|------|:----:|
| 单元 + 集成测试（208） | ✅ |
| GitHub Actions CI | ✅ |
| CLI（migration add/apply/revert/list/script） | ✅ |
| mdBook 用户文档 | ✅ |
| 性能基准 (criterion) | ✅ |

---

# 里程碑一：Beta 1 — CRUD 完整链路 + 查询完备性

**目标**: 用户可用 rust-ef 完成任意实体的增删改查，无需手写 SQL  
**整体进度: ~95%（SQLite）；~80%（PG/MySQL）**

## 1.1 通用 SaveChanges 实现

### 现状（v0.3.5）

- ✅ `DbContext` 通过 `SetOps<T>` + `ChangeExecutor` 实现通用 `save_changes()`
- ✅ 事务内遍历所有已注册实体类型
- ✅ 拦截器 `on_saving` / `on_saved` / `on_save_failed`
- ✅ `examples/blog` 使用现代 `discover_entities` + `ctx.set::<T>()` 模式

### 验收标准

- [x] 用户定义新实体后，`ctx.save_changes()` 自动持久化所有变更
- [x] INSERT 后自增主键正确回填（SQLite）
- [x] 任一步骤失败时事务回滚（代码路径已实现）
- [ ] **事务回滚集成测试**（insert → rollback → 验证不存在）
- [x] SQLite 内存库 CRUD 生命周期集成测试

---

## 1.2 条件表达式完备性

### 现状（v0.3.5）

- ✅ `BoolExpr` AST（Filter / Raw / And / Or / Not）
- ✅ `linq!` 宏：闭包表达式树 + 直接查询 + 可复用 expr
- ✅ `DbSet::filter(linq!(…))` / `QueryBuilder::filter(linq!(…))`
- ✅ LINQ 风格 IN：`ids.contains(b.field)`（非 `in_()`）

### 验收标准

- [x] `WHERE (a = 1 OR a = 2) AND b > 3` 正确生成 SQL
- [x] `WHERE id IN (1, 2, 3)` 参数化占位符
- [x] 表达式组合单元测试（`bool_expr_tests` + `linq_tests`，12+ 场景）
- [ ] `linq!` 省略闭包类型标注（类型推断）

---

## 1.3 DDL / 辅助操作

### 现状（v0.3.5）

- ✅ `DbSet::remove_range` / `load_all`
- ✅ `DbContext::ensure_created` / `ensure_deleted`
- ✅ `ModelBuilder::has_data` 种子数据
- [ ] `exists_by_id`（按 PK 检查存在，不返回实体）

### 验收标准

- [x] 集成测试：`ensure_created` → CRUD → `ensure_deleted`
- [ ] 复合主键实体完整 CRUD 集成测试（仅有 migration 单测）

---

## 1.4 SQLite 集成测试套件

文件: `crates/core/tests/sqlite_crud_tests.rs`

| 场景 | 状态 |
|------|:----:|
| CRUD 生命周期 | ✅ |
| 空表查询 | ✅ |
| IN / 聚合 / 分页 | ✅ |
| 种子数据 | ✅ |
| 事务回滚 | ❌ |
| 多实体同时 save_changes | ❌ |
| 复合主键 CRUD | ❌ |
| 全类型映射（bool/Option 等） | ⚠️ 部分 |

### 验收标准

- [x] 核心 CRUD 测试通过（9 个）
- [x] 不依赖外部数据库（`:memory:`）
- [ ] 6 项 spec 场景全部覆盖

---

## 1.5 Beta 1 新增：多 Provider 集成测试

文件:
- `crates/core/tests/postgres_crud_tests.rs`
- `crates/core/tests/mysql_crud_tests.rs`
- `crates/core/tests/common/mod.rs`（共享 CRUD 生命周期）

本地运行:
```bash
RUST_EF_PG_URL=postgres://user:pass@localhost/db cargo test -p rust-ef --test postgres_crud_tests
RUST_EF_MYSQL_URL=mysql://root:pass@localhost/db cargo test -p rust-ef --test mysql_crud_tests
```

CI 使用 GitHub Actions service containers 自动注入连接字符串。

### 验收标准

- [x] PostgreSQL CRUD 生命周期测试
- [x] MySQL CRUD 生命周期测试
- [x] CI matrix 含 PG + MySQL service

---

# 里程碑二：RC 1 — 导航/高级特性全功能就绪

**整体进度: ~95%**

## 2.1 Eager Loading 导航物化

### 现状（v0.3.5）

- ✅ 双查询策略（`navigation_loader.rs`）
- ✅ `INavigationSetter` derive 自动生成
- ✅ Include / ThenInclude 嵌套
- ✅ M2M 通过 Join 表加载

### 验收标准

- [x] HasMany Include 物化（`navigation_tests.rs`）
- [x] BelongsTo Include 物化
- [x] ThenInclude 嵌套
- [x] M2M 物化（`m2m_tests.rs`）

---

## 2.2 全局查询过滤器自动注入

### 现状（v0.3.5）

- ✅ `ModelBuilder::has_query_filter::<T>(sql)`
- ✅ `DbContext::set::<T>()` 创建 DbSet 时注入 `query_filter`
- ✅ `BoolExpr::Raw` 支持原始 SQL 片段

### 验收标准

- [x] 注册过滤器后所有 `query()` 自动附加 WHERE 条件
- [x] 全局过滤器 + `linq!` 组合的集成测试（`query_filter_exec_tests.rs`）
- [x] `query_ignore_filters()` 管理员查询
- [x] UPDATE/DELETE WHERE 同样受过滤器约束

---

## 2.3 乐观并发控制生效

### 现状（v0.5）

- ✅ `#[concurrency_check]` → `PropertyMeta.is_concurrency_token`
- ✅ `ChangeExecutor::execute_updates` WHERE 追加 `AND token_col = @original`
- ✅ `rows_affected == 0` → 返回 `ConcurrencyConflict`
- ✅ 6 个端到端测试（`concurrency_tests.rs`）

### 验收标准

- [x] 两并发连接修改同一实体，后者收到 `ConcurrencyConflict`
- [x] UPDATE 使用原始 token 快照
- [x] 无并发修改时 DELETE/UPDATE 正常成功

---

## 2.4 CLI Migration 连接数据库

### 现状（v0.5）

- ✅ `crates/cli/` CLI crate 已实现
- ✅ 库级 `MigrationEngine::apply()` / `apply_pending()` / `revert()` / `revert_last()` / `revert_to_target()`
- ✅ 读取 `__ef_migrations_history` 跳过已应用迁移
- ✅ `generate_script(from, to)` 生成前向/反向 SQL 脚本

### 支持的命令

```bash
rust-ef-cli add InitialCreate --output ./Migrations
rust-ef-cli list --connection "..." --provider sqlite|postgres|mysql
rust-ef-cli apply --connection "..." --provider sqlite|postgres|mysql
rust-ef-cli revert --connection "..." --target PreviousMigration
rust-ef-cli script --from X --to Y   # 或 --name SingleMigration
```

### 验收标准

- [x] `migration apply` 成功执行并记录 history
- [x] `migration revert` 回滚最近一次或回滚到指定迁移
- [x] `migration script` 生成前向/反向 SQL
- [x] 13 个 CLI 单元测试（`migration_cli_tests.rs`）

---

## 2.5 RC 1 新增：迁移 FK/索引 diff

### 现状（v0.5）

- ✅ `SchemaChange::AddForeignKey` / `DropForeignKey` 已接入 `diff()`
- ✅ `SchemaChange::CreateIndex` / `DropIndex` 已接入 `diff()`
- ✅ `SnapshotColumn` 包含 `has_index` / `is_unique` 字段
- ✅ `columns_structurally_equal` 排除索引字段，避免误报 AlterColumn
- ✅ MySQL DROP INDEX 方言差异已处理
- ✅ 10 个索引 diff 测试（`index_diff_tests.rs`）

### 验收标准

- [x] 新增 FK 列时生成 `ALTER TABLE ... ADD CONSTRAINT`
- [x] 删除 FK 时生成对应 DROP
- [x] 新增索引生成 `CREATE [UNIQUE] INDEX`
- [x] 删除索引生成 `DROP INDEX`（SQLite/PG: `IF EXISTS`；MySQL: `ON table`）
- [x] 索引变更不产生多余 AlterColumn

---

# 里程碑三：1.0 — 文档 / CI / 性能 / 安全

**整体进度: ~30%**

## 3.1 完整用户文档

### 现状

- ✅ 根目录 `README.md`（架构、Quick Start）
- ⚠️ Provider README 仍引用旧名 `lref` / `lref-provider-*`
- ⚠️ `examples/blog` 未展示现代 type-map DbContext
- ❌ mdBook / API 参考 / 迁移指南

### 需求

- [ ] 更新所有 crate README 为 `rust-ef-*` 命名
- [ ] 重写 blog 示例使用 `add_dbcontext` + `ctx.set::<T>()`
- [ ] `docs/book/` mdBook 项目
- [ ] 修正 README 中 CLI 声明（实现前标注「计划中」）

---

## 3.2 CI/CD Pipeline

文件: `.github/workflows/ci.yml`

```yaml
jobs:
  lint:
    # fmt --check + clippy（默认 + chrono/uuid/decimal features）-D warnings
  test:
    strategy:
      fail-fast: false
      matrix:
        db: [sqlite, postgres, mysql]
    services:   # postgres:16 + mysql:8 service containers
    steps:
      # sqlite  → cargo test --features chrono,uuid,decimal -- --skip postgres --skip mysql
      # postgres→ cargo test --features chrono,uuid,decimal --test postgres_crud_tests
      # mysql   → cargo test --features chrono,uuid,decimal --test mysql_crud_tests
```

### 验收标准

- [x] PR 触发 CI，三库 matrix（PG/MySQL 用 service container）
- [x] Clippy 零 warning（`-D warnings`，默认 + chrono/uuid/decimal features 双路径）
- [x] fmt 检查独立 lint job
- [x] feature 门控测试在 CI 中运行（extended_types_tests）

---

## 3.3 类型扩展

### 现状（v0.5）

`DbValue` 核心 9 变体保持不变（Null/Bool/I16/I32/I64/F32/F64/String/Bytes），通过可选 feature 在 `String` 变体上承载 chrono/uuid/decimal 文本表示，避免破坏既有 ABI。

### 已实现

- [x] `chrono` feature：`DateTime<Utc>` / `NaiveDateTime` / `NaiveDate` 映射（RFC3339 / `"YYYY-MM-DD HH:MM:SS"` / `"YYYY-MM-DD"`）
- [x] `uuid` feature：`uuid::Uuid` 类型支持（含 `v4`）
- [x] `decimal` feature：`rust_decimal::Decimal` 高精度小数
- [x] 三方方言 DDL 映射（PG `TIMESTAMPTZ`/`UUID`/`NUMERIC`；MySQL `DATETIME`/`CHAR(36)`/`DECIMAL(38,18)`；SQLite 统一 `TEXT`）
- [x] 6 个集成测试（`extended_types_tests.rs`，feature 组合门控）

### 待完善

- [ ] Provider 原生参数绑定（目前经 `String` 中转，PostgreSQL 原生 `TIMESTAMPTZ`/`UUID` 参数为后续优化项）

---

## 3.4 性能基准（可选）

**状态: ✅ 已完成**

文件: `crates/core/benches/bench_insert.rs`, `bench_query.rs`, `bench_include.rs`

使用 `criterion`（`async_tokio` feature）对 SQLite 内存库进行基准测试：

- **bench_insert** — 批量 INSERT 吞吐量（100 / 500 / 1000 行，单次 `save_changes` 事务）
- **bench_query** — 批量 SELECT 吞吐量（`to_list` 全表 + `linq!` 过滤，100 / 500 / 1000 行）
- **bench_include** — Include 预加载 vs N+1 查询对比（50 blogs × 10 posts）

运行: `cargo bench -p rust-ef`

> 注: "与 sqlx 裸写对比基线" 未纳入，因为 rust-ef 的 SQLite provider 内部已使用 sqlx，对比意义有限。

---

## 3.5 已移除 / 不再规划

| 项 | 说明 |
|----|------|
| `DbSetCollection` | 已被 type-map + `SetOps` 取代 |
| `DbCache` / Identity Map | 已从 core 移除 |
| `filter!` 宏 | 已移除，统一 `linq!` |
| `save_changes_all!` | 已移除 |

---

# 验收矩阵（v0.5 快照）

| 能力 | v0.2 Alpha | v0.3.5 | v0.5 当前 | 1.0 |
|------|:----------:|:-----------:|:---------:|:---:|
| 通用 SaveChanges | 手写 | ✅ 自动 | ✅+并发 | ✅+并发 |
| WHERE 表达式 | AND only | ✅ linq! | ✅+子查询 | — |
| 导航 Eager Loading | SQL only | ✅ 物化 | ✅ | 缓存 |
| M2M | ❌ | ✅ | ✅ | ✅ |
| 全局过滤器 | 注册 | ✅ 注入 | ✅+ignore | ✅ |
| 乐观并发 | 元数据 | 元数据 | ✅ 生效 | 测试 |
| CLI Migration | ❌ | ❌ | ✅ 三库 | 三库 |
| Provider 集成测试 | SQLite | SQLite | ✅ 三库 | 三库 |
| chrono/uuid/decimal | ❌ | ❌ | ✅ 可选 feature | 原生参数 |
| 测试数量 | 19 | 46 | **208** | 200+ |
| CI | ❌ | ❌ | ✅ 三库 matrix | 三库 |
| 文档 | 计划 | README | ✅ mdBook | mdBook |

---

# 实现优先级（2026-06-26 起）

```
已完成 (v0.5):
  ✅ 1.5 PostgreSQL + MySQL 集成测试
  ✅ 2.3 乐观并发生效
  ✅ 2.4 CLI crate（add/apply/revert/list/script）
  ✅ 2.5 迁移 FK/索引 diff
  ✅ 子查询/关联过滤（any/none/all）
  ✅ 全局查询过滤器 + query_ignore_filters
  ✅ 软删除/审计拦截器示例 + 文档
  ✅ 3.3 chrono / uuid / decimal 类型支持（可选 feature）
  ✅ 3.2 GitHub Actions CI 三库 matrix（lint + sqlite/pg/mysql）
  ✅ 1.2 linq! 类型推断（已调研并文档化，proc_macro 根本限制）
  ✅ 1.3 exists_by_id / exists_by_key（SELECT 1 ... LIMIT 1）
  ✅ 1.4 事务回滚 + 复合主键 CRUD 集成测试
  ✅ 3.4 性能基准（criterion: 批量 INSERT / SELECT / Include vs N+1）

P0 — 1.0 GA blocker:
  （无）

P1 — 1.0 polish:
  Lazy Loading（可选）
  Provider 原生 chrono/uuid 参数绑定（目前经 String 中转）
```

---

# 已知限制（v0.5 使用者须知）

1. **`linq!` 需显式类型**：`|b: Blog|`，暂不支持省略
2. **无 Lazy Loading**：必须显式 `include`
3. **拦截器只读**：`SaveChangesContext` 不含实体引用，无法在拦截器中改字段；软删除/时间戳需手动标记
4. **`from_row` 基于 `Vec<String>`**：大结果集性能与类型安全有限
5. **DbContext DI 为 Transient**：长生命周期场景需自行管理 scope
6. **chrono/uuid/decimal 经 `String` 中转**：可选 feature 已支持类型映射，但 Provider 参数绑定仍走文本通道，未利用 PG 原生 `TIMESTAMPTZ`/`UUID` 参数类型
7. **无 CTE / Window 函数**：复杂分析查询需退回原始 SQL

---

# 附录：测试清单（当前 208 个）

| 文件 | 数量 | 覆盖 |
|------|:----:|------|
| `integration_tests.rs` | 16 | 元数据、迁移方言、QueryState SQL |
| `sqlite_crud_tests.rs` | 9 | 端到端 CRUD |
| `linq_tests.rs` | 7 | linq! 宏 |
| `bool_expr_tests.rs` | 5 | BoolExpr SQL |
| `advanced_tests.rs` | 5 | ChangeTracker、迁移生成 |
| `navigation_tests.rs` | 2 | Include / ThenInclude |
| `m2m_tests.rs` | 2 | Many-to-Many |
| `concurrency_tests.rs` | 6 | 乐观并发冲突检测 |
| `query_filter_exec_tests.rs` | 4 | 全局过滤器 UPDATE/DELETE 约束 |
| `subquery_tests.rs` | 8 | any/none/all 子查询 |
| `migration_cli_tests.rs` | 13 | revert_to_target / generate_script |
| `index_diff_tests.rs` | 10 | CreateIndex / DropIndex diff |
| `model_builder_cache_tests.rs` | — | OnceLock 元数据缓存 |
| `batch_dml_tests.rs` | — | 批量 INSERT / DELETE |
| `navigation_perf_tests.rs` | — | NavigationLoader 优化 |
| `connection_pool_tests.rs` | — | 连接池配置 |
| `extended_types_tests.rs` | 6 | chrono/uuid/decimal 类型映射（feature 门控） |
| `exists_by_id_tests.rs` | 8 | exists_by_id / exists_by_key（单主键 + 复合主键） |
| `transaction_composite_tests.rs` | 6 | 事务回滚 + 复合主键 CRUD 全生命周期 |
| 其他（单元 + 集成） | 100+ | 类型映射、DI、拦截器等 |

---

*下次审计建议触发条件：Lazy Loading 实现、Provider 原生 chrono/uuid 参数绑定、或版本升至 1.0。*
