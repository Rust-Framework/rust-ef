# rust-ef 生产就绪技术规格说明书

> 版本: v0.3.5 — 基于 2026-06-25 审计结果  
> 包名: `rust-ef`（workspace: `crates/core`）  
> 目标: 逐步推进至 v1.0 生产就绪状态  
> **当前阶段: RC 1 进行中（约 78% 就绪度）**

---

## 执行摘要

rust-ef v0.3.5 已具备 EF Core 风格 ORM 的**核心骨架**：类型映射式 `DbContext`、通用 `save_changes()`、`linq!` 查询 DSL、导航 Include、M2M、迁移引擎库 API、DI 集成。在 **SQLite** 上有完整的 CRUD 集成测试（46 个测试全绿）。

**尚不具备通用生产条件**，主要缺口：CLI 工具、PostgreSQL/MySQL 集成验证、乐观并发、完整迁移 history 工作流、CI 流水线。

| 场景 | 建议 |
|------|------|
| SQLite 原型 / 内部工具 | 可用，需了解限制 |
| PostgreSQL / MySQL 生产 | 需自行集成测试后再用 |
| 多写并发 + 乐观锁 | 不可用 |
| 团队迁移 CLI 工作流 | 不可用 |

---

## 里程碑总览

```
Alpha 2 (35%) ──► 当前 v0.3.5 (~60%) ──► Beta 1 ──► RC 1 ──► 1.0
                      ↑
              Beta1 核心项大部分完成
              RC1 部分完成；运维项仍缺
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

### 查询

| 能力 | 状态 | 说明 |
|------|:----:|------|
| `BoolExpr`（AND/OR/NOT/Raw） | ✅ | `query.rs` |
| `linq!` 表达式树 | ✅ | `crates/macros/src/linq.rs` |
| IN / BETWEEN / IS NULL / contains | ✅ | linq + QueryBuilder |
| join / group_by / having / 分页 | ✅ | QueryBuilder |
| 全局 Query Filter 自动注入 | ✅ | `ModelBuilder::has_query_filter` |
| 子查询 / 关联过滤 | ❌ | 未规划 |

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
| Revert 工作流 | ✅ | `revert` / `revert_last` |
| FK/索引 diff | ✅ | `AddForeignKey` / `DropForeignKey` 已接入 diff |

### Provider

| Provider | 实现 | 集成测试 |
|----------|:----:|:--------:|
| SQLite (`rust-ef-sqlite`) | ✅ | ✅ 9 CRUD + 导航/M2M |
| PostgreSQL (`rust-ef-postgres`) | ✅ | ✅ 可选（`RUST_EF_PG_URL` / CI） |
| MySQL (`rust-ef-mysql`) | ✅ | ✅ 可选（`RUST_EF_MYSQL_URL` / CI） |

### 工程化

| 能力 | 状态 |
|------|:----:|
| 单元 + 集成测试（62） | ✅ |
| GitHub Actions CI | ✅ |
| CLI（migration / scaffold） | ✅ |
| mdBook 用户文档 | ❌ |
| 性能基准 | ❌ |

---

# 里程碑一：Beta 1 — CRUD 完整链路 + 查询完备性

**目标**: 用户可用 rust-ef 完成任意实体的增删改查，无需手写 SQL  
**整体进度: ~85%（SQLite）；~40%（PG/MySQL）**

## 1.1 通用 SaveChanges 实现

### 现状（v0.3.5）

- ✅ `DbContext` 通过 `SetOps<T>` + `ChangeExecutor` 实现通用 `save_changes()`
- ✅ 事务内遍历所有已注册实体类型
- ✅ 拦截器 `on_saving` / `on_saved` / `on_save_failed`
- ⚠️ `examples/blog` 仍使用旧式手写 `save_one_set`（示例未更新）

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

**整体进度: ~70%**

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
- [ ] 全局过滤器 + `linq!` 组合的集成测试

---

## 2.3 乐观并发控制生效

### 现状（v0.3.5）

- ✅ `#[concurrency_check]` → `PropertyMeta.is_concurrency_token`
- ❌ `ChangeExecutor::execute_updates` WHERE 仅用主键
- ❌ `EFError::ConcurrencyConflict` 从未触发

### 需求规格

1. Modified 实体 UPDATE 时，WHERE 追加 `AND token_col = @original`
2. `rows_affected == 0` → 返回 `ConcurrencyConflict`
3. 成功时可选更新 token 值

### 验收标准

- [ ] 两并发连接修改同一实体，后者收到 `ConcurrencyConflict`
- [ ] UPDATE 使用原始 token 快照

---

## 2.4 CLI Migration 连接数据库

### 现状（v0.3.5）

- ❌ **无 CLI crate**（README 宣称存在，与实际不符）
- ✅ 库级 `MigrationEngine::apply()` 可执行单迁移
- ⚠️ 无读取 history 跳过已应用迁移的逻辑
- ⚠️ 无 revert 命令

### 需求规格

新建 `crates/cli/`（或独立 binary crate `rust-ef-cli`）：

```bash
rust-ef migration add InitialCreate --output ./Migrations
rust-ef migration list --connection "postgres://localhost/app"
rust-ef migration apply --connection "postgres://localhost/app"
rust-ef migration revert --connection "..." --target PreviousMigration
rust-ef migration script --from X --to Y
rust-ef scaffold dbcontext --connection "..." --output ./Entities
```

实现要点：

1. Provider 连接 + `ensure_history_table`
2. 读取 `__ef_migrations_history` 获取已应用列表
3. 按序执行未应用迁移 Up SQL + 写入 history
4. Revert 执行 Down SQL + 删除 history 记录

### 验收标准

- [ ] `migration apply` 在 PostgreSQL 中成功执行并记录 history
- [ ] `migration revert` 回滚最近一次迁移
- [ ] SQLite / MySQL 同等验证

---

## 2.5 RC 1 新增：迁移 FK/索引 diff

### 现状

- `SchemaChange::AddForeignKey` / `DropForeignKey` 已定义但未接入 `diff()`
- 增量迁移无法处理外键变更

### 验收标准

- [ ] 新增 FK 列时生成 `ALTER TABLE ... ADD CONSTRAINT`
- [ ] 删除 FK 时生成对应 DROP

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
  test:
    strategy:
      matrix:
        db: [sqlite, postgres, mysql]
    steps:
      - cargo test --workspace
      - cargo clippy --workspace -- -D warnings
      - cargo fmt --check
```

### 验收标准

- [ ] PR 触发 CI，三库 matrix（PG/MySQL 用 service container）
- [ ] Clippy 零 warning（当前 lib 有 3 个 warning）

---

## 3.3 类型扩展

### 现状

`DbValue` 仅支持: Null / 数值 / bool / String / Bytes

### 需求

- [ ] `chrono` feature：DateTime / NaiveDate 映射
- [ ] UUID 类型
- [ ] Decimal（rust_decimal 可选）

---

## 3.4 性能基准（可选）

文件: `benches/crud_benchmark.rs`

- 批量插入 1000 条吞吐量
- Include vs N+1 对比
- 与 sqlx 裸写对比基线

---

## 3.5 已移除 / 不再规划

| 项 | 说明 |
|----|------|
| `DbSetCollection` | 已被 type-map + `SetOps` 取代 |
| `DbCache` / Identity Map | 已从 core 移除 |
| `filter!` 宏 | 已移除，统一 `linq!` |
| `save_changes_all!` | 已移除 |

---

# 验收矩阵（v0.3.5 快照）

| 能力 | v0.2 Alpha | v0.3.5 当前 | Beta 1 | RC 1 | 1.0 |
|------|:----------:|:-----------:|:------:|:----:|:---:|
| 通用 SaveChanges | 手写 | ✅ 自动 | ✅ | ✅ | ✅+并发 |
| WHERE 表达式 | AND only | ✅ linq! | ✅ | 子查询 | — |
| 导航 Eager Loading | SQL only | ✅ 物化 | — | ✅ | 缓存 |
| M2M | ❌ | ✅ | — | ✅ | ✅ |
| 全局过滤器 | 注册 | ✅ 注入 | — | ✅ | ✅ |
| 乐观并发 | 元数据 | 元数据 | 元数据 | 生效 | 测试 |
| CLI Migration | ❌ | ❌ | 本地生成 | 连接 DB | 三库 |
| Provider 集成测试 | SQLite | SQLite | 三库 | 三库 | 三库 |
| 测试数量 | 19 | **46** | 50+ | 60+ | 80+ |
| CI | ❌ | ❌ | SQLite | 三库 | 三库 |
| 文档 | 计划 | README | API | 指南 | mdBook |

---

# 实现优先级（2026-06-25 起）

```
P0 — 生产 blocker（建议 4–6 周）:
  1.5 PostgreSQL + MySQL 集成测试
  2.4 CLI crate（migration add/apply/list/revert）
  3.2 GitHub Actions CI

P1 — RC 必备（建议 2–4 周）:
  2.3 乐观并发生效
  1.4  事务回滚 + 复合主键集成测试
  2.5 迁移 FK diff
  3.1 文档与示例对齐（去 lref 旧名、更新 blog 示例）

P2 — 1.0  polish:
  1.2  linq! 类型推断
  1.3  exists_by_id
  3.3  DateTime / UUID 类型
  3.4  性能基准
```

---

# 已知限制（v0.3.5 使用者须知）

1. **`linq!` 需显式类型**：`|b: Blog|`，暂不支持省略
2. **无 Lazy Loading**：必须显式 `include`
3. **无子查询 / 关联过滤**：不能写 `b.posts.any(p => p.title.contains("x"))`
4. **`from_row` 基于 `Vec<String>`**：大结果集性能与类型安全有限
5. **DbContext DI 为 Transient**：长生命周期场景需自行管理 scope
6. **迁移 apply 不查 history**：重复 apply 可能失败，需 CLI 补齐
7. **README CLI 声明超前于实现**：以本文档为准

---

# 附录：测试清单（当前 46 个）

| 文件 | 数量 | 覆盖 |
|------|:----:|------|
| `integration_tests.rs` | 16 | 元数据、迁移方言、QueryState SQL |
| `sqlite_crud_tests.rs` | 9 | 端到端 CRUD |
| `linq_tests.rs` | 7 | linq! 宏 |
| `bool_expr_tests.rs` | 5 | BoolExpr SQL |
| `advanced_tests.rs` | 5 | ChangeTracker、迁移生成 |
| `navigation_tests.rs` | 2 | Include / ThenInclude |
| `m2m_tests.rs` | 2 | Many-to-Many |

---

*下次审计建议触发条件：CLI 合并、PG/MySQL 集成测试落地、或版本升至 0.4.0。*
