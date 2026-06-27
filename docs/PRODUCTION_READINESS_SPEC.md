# rust-ef 生产就绪技术规格说明书

> 版本: v1.1 — 基于 2026-06-27 v1.1 Query Fidelity 审计结果（含发布评审修复）  
> 包名: `rust-ef`（workspace: `crates/core`）  
> 目标: v1.1 查询保真度迭代  
> **当前阶段: v1.1 已达成（Lazy Loading + IN/NOT IN 子查询 + CTE/Window 函数 + PG 方言修复 + 发布评审通过）**

---

## 执行摘要

rust-ef v1.1 已具备 EF Core 风格 ORM 的**完整生产就绪能力**：类型映射式 `DbContext`、通用 `save_changes()`、`linq!` 查询 DSL、导航 Include、M2M、迁移引擎库 API + CLI 工具、DI 集成、子查询/关联过滤、乐观并发、全局查询过滤器、SaveChanges 拦截器、chrono/uuid/decimal 可选类型支持、exists_by_id/exists_by_key 存在性检查、事务回滚与复合主键 CRUD 集成测试。v1.1 新增 Lazy Loading（opt-in）、IN/NOT IN 标量子查询、CTE 与 Window 函数支持（含 `linq!(with ...)` 语法糖），并修复 PostgreSQL HAVING 占位符、LIMIT/OFFSET 方言及多 typed CTE `$N` 占位符冲突 bug。在 **SQLite / PostgreSQL / MySQL** 上有完整的 CRUD 集成测试（272 个测试全绿），CI 三库 matrix 已就位。mdBook 在线文档已部署至 GitHub Pages，安全审计通过，API 稳定无 deprecated 残留，Criterion 性能基准就绪。

**v1.1 全部验收标准通过**，剩余项为 v1.2+ 范围的规模扩展（L2 缓存、读写分离、分库分表）与生态集成（GraphQL）。

| 场景 | 建议 |
|------|------|
| SQLite 原型 / 内部工具 | ✅ 可用 |
| PostgreSQL / MySQL 生产 | ✅ 可用（需自行集成测试验证） |
| 多写并发 + 乐观锁 | ✅ 可用（token 冲突检测） |
| 团队迁移 CLI 工作流 | ✅ 可用（add/apply/revert/list/script） |
| 生产文档查阅 | ✅ mdBook 在线（GitHub Pages 自动部署） |

---

## 里程碑总览

```
Alpha 2 (35%) ──► v0.3.5 (~60%) ──► Beta 1 (~85%) ──► v0.5 RC 1 (~98%) ──► v1.0 GA (100%) ──► 当前 v1.1 (查询保真度)
                                                                                                              ↑
                                                                  全部验收标准通过
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
| Lazy Loading | ✅ | opt-in（`use_lazy_loading(true)`） |

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
| 单元 + 集成测试（209） | ✅ |
| GitHub Actions CI | ✅ |
| CLI（migration add/apply/revert/list/script） | ✅ |
| mdBook 用户文档（GitHub Pages 自动部署） | ✅ |
| 性能基准 (criterion) | ✅ |
| 安全审计 | ✅ |
| API 稳定（无 deprecated 残留） | ✅ |
| CHANGELOG.md | ✅ |

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

# 里程碑三：1.0 GA — 文档 / CI / 性能 / 安全 / 稳定 API

**整体进度: 100% ✅**

## 3.1 完整用户文档

### 现状（v1.0 GA ✅）

- ✅ 根目录 `README.md`（架构、Quick Start、最佳实践、版本号 1.0）
- ✅ 所有 crate README 统一品牌为 `rust-ef-*`（v0.4 Beta 1 已完成）
- ✅ `examples/blog` 使用现代 type-map DbContext + `add_dbcontext`
- ✅ mdBook 项目 `docs/rust-ef/book.toml` + `SUMMARY.md`（11 章节 + 前言 + 附录）
- ✅ GitHub Pages 自动部署 `.github/workflows/docs.yml`（`peaceiris/action-mdbook` + `actions/deploy-pages@v4`）
- ✅ 文档搜索、暗色主题（navy）、章节折叠
- ✅ 在线访问地址: https://rf2026.github.io/rust-ef/

### 验收标准

- [x] 所有 crate README 品牌统一为 `rust-ef-*`
- [x] `examples/blog` 使用 `add_dbcontext` + `ctx.set::<T>()`
- [x] mdBook 项目可构建
- [x] GitHub Pages 自动部署
- [x] 文档搜索功能启用

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

## 3.5 安全审计

**状态: ✅ 已通过（v1.0 GA）**

文件: `docs/rust-ef/11-best-practices/security.md`

### 审计结论

✅ **无 SQL 注入漏洞**。所有运行时值通过 `DbValue` 参数化，driver 层 `ToSql` / `bind` 完成占位符绑定；`format!` 仅用于标识符、占位符、DDL，且全部来源于编译期实体元数据。`BoolExpr::Raw` 仅在内部使用硬编码 `"1=1"`，无用户可达 API。

### 审计覆盖

| 主题 | 状态 | 说明 |
|------|:----:|------|
| SQL 注入防护 | ✅ | 运行时值全部参数化；标识符来自编译期元数据 |
| 迁移脚本安全 | ✅ | DDL 不可参数化属设计信任模型，文档已说明 |
| 连接字符串安全 | ✅ | 存储/口令保护/TLS 与 NoTls 取舍文档化 |
| 敏感字段映射 | ✅ | 密码哈希、投影过滤建议已文档化 |
| 全局查询过滤器与多租户 | ✅ | `has_query_filter` + `query_ignore_filters` 配合 |
| 生产部署加固清单 | ✅ | 6 项 checklist 已提供 |

### 加固建议（非漏洞）

下列项已在 `security.md` 中以「加固建议」形式记录，未纳入 1.0 GA blocker：

- `quote_identifier` 不转义嵌入式引号（标识符来自编译期元数据，不可利用）
- `HavingExpr::to_sql` 使用 `?` 而非 `gen.parameter_placeholder(index)`（PostgreSQL 正确性 bug，非安全问题）
- PostgreSQL Provider 默认 `NoTls`（部署加固，非框架漏洞）

### 验收标准

- [x] SQL 注入审查：所有运行时值参数化
- [x] 连接字符串安全处理文档化
- [x] 敏感字段（密码）映射最佳实践
- [x] 文档化安全指南（`security.md`）

---

## 3.6 稳定 API 与 1.0 发布

**状态: ✅ 已完成（v1.0 GA）**

### API 稳定性

- ✅ 公共 API 全部稳定，无 `#[deprecated]` 残留
- ✅ 历史 `LrefError` / `LrefResult` 别名已从 `crates/core/src/error.rs` 移除（无任何引用）
- ✅ 统一命名为 `EFError` / `EFResult`
- ✅ 工作区版本号统一升至 `1.0.0`，所有 crate 间依赖同步

### 版本号升级

| 文件 | 变更 |
|------|------|
| `Cargo.toml` (workspace) | `version = "0.3.5"` → `"1.0.0"` |
| `crates/core/Cargo.toml` | `rust-ef-macros = "1.0.0"` |
| `crates/sqlite/Cargo.toml` | `rust-ef = "1.0.0"` |
| `crates/postgres/Cargo.toml` | `rust-ef = "1.0.0"` |
| `crates/mysql/Cargo.toml` | `rust-ef = "1.0.0"` |
| `README.md` Quick Start | `rust-ef = "0.3"` → `"1.0"`；`rust-ef-sqlite = "0.3"` → `"1.0"` |

### CHANGELOG

- ✅ `CHANGELOG.md` 创建于工作区根目录，记录 v0.1 → v1.0 全部变更
- ✅ 遵循 [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) 格式
- ✅ 含 7 个版本条目（v0.1 / v0.2 / v0.3 / v0.3.5 / v0.4 / v0.5 / v1.0.0）

### 验证

```
cargo clippy --workspace --all-features -- -D warnings    ✅ 0 warnings
cargo fmt --all -- --check                                 ✅ pass
cargo test --workspace --all-features --no-fail-fast       ✅ 209 tests pass
cargo bench --workspace --no-run                           ✅ 3 benches compile
```

### 1.0 GA 验收标准

| 标准 | 状态 |
|------|:----:|
| chrono + uuid 类型支持 | ✅ |
| mdBook 文档在线可访问 | ✅ |
| 性能基准测试报告 | ✅ |
| 安全审计通过 | ✅ |
| API 稳定，无 deprecated 残留 | ✅ |
| 示例项目 ≥ 3 个 | ✅（`blog`、`soft_delete`、`audit`） |
| 1.0.0 版本发布 | ✅ |

---

## 3.7 已移除 / 不再规划

| 项 | 说明 |
|----|------|
| `DbSetCollection` | 已被 type-map + `SetOps` 取代 |
| `DbCache` / Identity Map | 已从 core 移除 |
| `filter!` 宏 | 已移除，统一 `linq!` |
| `save_changes_all!` | 已移除 |

---

## 3.8 v1.1 发布评审（2026-06-27）

**状态: ✅ 通过**

发布评审在 v1.1 CTE 语法糖实现完成后进行，目标是确保发布后稳定且高效。评审包含两个并行子任务：框架审计（7 项准则）+ 代码审查（架构级 bug + 可维护性）。

### 评审准则与结果

| 准则 | 结果 | 说明 |
|------|:----:|------|
| 测试全绿 | ✅ | 272 个测试通过（含 4 个新增 PG CTE 回归测试） |
| Clippy 零 warning | ✅ | `cargo clippy --workspace --all-features -- -D warnings` |
| fmt 一致 | ✅ | `cargo fmt --all` 应用了所有格式修正 |
| Bench 编译 | ✅ | `cargo bench --workspace --no-run` 3 个基准可执行 |
| 无 deprecated 残留 | ✅ | 公共 API 全部稳定 |
| API 表面完整 | ✅ | prelude 导出 `WindowFuncKind` / `WindowSpec` / `CteSpec` |
| 文档一致 | ✅ | CHANGELOG / SPEC / 内联文档同步 v1.1 |

### 代码审查发现与修复

| 严重度 | 问题 | 状态 |
|:------:|------|:----:|
| 🔴 CRITICAL | 多 typed CTE `cte_idx` 在 `.map()` 闭包内重置为 1，导致 PostgreSQL `$N` 占位符在多个 typed CTE 间冲突（`$1, $1` 而非 `$1, $2`） | ✅ 修复 |
| 🟡 中 | `CteSpec` 缺少 `#[non_exhaustive]`，未来字段添加会破坏 semver | ✅ 修复 |
| 🟡 中 | `to_sql_with` CTE 注释误导（"param_idx starts at 1 for this CTE"） | ✅ 修复 |
| 🟢 低 | CHANGELOG 缺少 `[1.1.0]:` 链接引用 | ✅ 修复 |
| 🟢 低 | 缺少 PostgreSQL 多 typed CTE 测试覆盖 | ✅ 新增 4 个回归测试 |

### 修复实现

**CRITICAL bug 修复**（[query.rs:938-982](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L938-L982)）：将 `.map()` 闭包改为 `for` 循环，引入 `running_idx: usize` 在 CTE 间累加。typed 模式下 `cte_idx` 从 `running_idx` 起始，编译完成后回写；raw 模式下按 `params.len()` 推进以保持与 `all_params()` 顺序一致。

**回归测试**（[cte_syntax_tests.rs:340-481](file:///e:/GitCode/RF/rust-ef/crates/core/tests/cte_syntax_tests.rs#L340-L481)）：4 个 PostgreSQL 方言测试，使用 `PgLikeGenerator` mock（无需 live PG）：
- `test_pg_single_typed_cte_uses_dollar_n` — 单 CTE 产出 `$1`
- `test_pg_multiple_typed_ctes_contiguous_placeholders` — 双 CTE 产出 `$1, $2`（回归核心）
- `test_pg_multi_cte_with_main_where_contiguous` — 三参数连续 `$1, $2, $3` 跨 CTE + 主 WHERE
- `test_pg_compound_where_cte_placeholder_count` — 单 CTE 复合 WHERE 产出 `$1, $2` 且无 `$3`

### 验收标准

- [x] 7 项评审准则全部通过
- [x] CRITICAL bug 已修复并有回归测试覆盖
- [x] 中低严重度问题全部修复
- [x] 272 个测试全绿，clippy/fmt/bench 全部通过
- [x] CHANGELOG 与 SPEC 同步更新

---

# 验收矩阵（v1.0 GA 快照）

| 能力 | v0.2 Alpha | v0.3.5 | v0.5 RC 1 | v1.0 GA | v1.1 |
|------|:----------:|:-----------:|:---------:|:---:|:---:|
| 通用 SaveChanges | 手写 | ✅ 自动 | ✅+并发 | ✅+并发 | ✅+并发 |
| WHERE 表达式 | AND only | ✅ linq! | ✅+子查询 | ✅+子查询 | ✅+IN/NOT IN 子查询 |
| 导航 Eager Loading | SQL only | ✅ 物化 | ✅ | ✅ | ✅ |
| 导航 Lazy Loading | ❌ | ❌ | ❌ | ❌ | ✅ opt-in |
| M2M | ❌ | ✅ | ✅ | ✅ | ✅ |
| 全局过滤器 | 注册 | ✅ 注入 | ✅+ignore | ✅ | ✅ |
| 乐观并发 | 元数据 | 元数据 | ✅ 生效 | ✅ 生效（6 测试） | ✅ 生效（6 测试） |
| CLI Migration | ❌ | ❌ | ✅ 三库 | ✅ 三库 | ✅ 三库 |
| Provider 集成测试 | SQLite | SQLite | ✅ 三库 | ✅ 三库 | ✅ 三库 |
| chrono/uuid/decimal | ❌ | ❌ | ✅ 可选 feature | ✅ 可选 feature | ✅ PG 原生绑定 |
| CTE / Window 函数 | ❌ | ❌ | ❌ | ❌ | ✅ 10 种窗口函数 + CTE（含 `linq!(with ...)` 语法糖） |
| PG HAVING/LIMIT 修复 | ❌ | ❌ | ❌ | ❌ | ✅ |
| PG 多 typed CTE `$N` 修复 | ❌ | ❌ | ❌ | ❌ | ✅ |
| 测试数量 | 19 | 46 | 208 | 209 | **272** |
| CI | ❌ | ❌ | ✅ 三库 matrix | ✅ 三库 matrix | ✅ 三库 matrix |
| 文档 | 计划 | README | ✅ mdBook | ✅ mdBook + GitHub Pages | ✅ mdBook + GitHub Pages |
| 性能基准 | ❌ | ❌ | ❌ | ✅ criterion（3 benches） | ✅ criterion（3 benches） |
| 安全审计 | ❌ | ❌ | ❌ | ✅ 通过 | ✅ 通过 |
| API 稳定 / 无 deprecated | ❌ | ❌ | ❌ | ✅ | ✅ |
| CHANGELOG | ❌ | ❌ | ❌ | ✅ v0.1 → v1.0 | ✅ v0.1 → v1.1 |
| 版本号 | 0.1 | 0.3.5 | 0.5 | 1.0.0 | **1.1.0** |

---

# 实现优先级（2026-06-27 起，v1.1 已达成）

```
已完成 (v1.0 GA):
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
  ✅ 3.1 mdBook 用户文档 + GitHub Pages 自动部署
  ✅ 3.4 性能基准（criterion: 批量 INSERT / SELECT / Include vs N+1）
  ✅ 3.5 安全审计（无 SQL 注入漏洞；security.md 指南发布）
  ✅ 3.6 稳定 API + 1.0.0 版本发布（无 deprecated 残留、CHANGELOG 完成）

已完成 (v1.1 Query Fidelity):
  ✅ 3.1 Lazy Loading（ILazyInit trait + LazyContext + opt-in use_lazy_loading）
  ✅ 3.2 PostgreSQL 原生 chrono/uuid 参数绑定（DbValue 保留原生类型）
  ✅ 3.3 CTE / Window 函数（WindowSpec AST + CteSpec + linq! window 子句）
  ✅ 3.4 HAVING/LIMIT 方言 bug 修复（PG 占位符 + pagination 委托）
  ✅ 3.5 IN / NOT IN 标量子查询（InSubquerySpec + linq! in_subquery 语法）

v1.1 已完成:
  ✅ Lazy Loading（导航属性延迟加载，opt-in via use_lazy_loading）
  ✅ Provider 原生 chrono/uuid 参数绑定（PG 原生类型不再经 String 中转）
  ✅ IN / NOT IN 标量子查询（b.field.in_subquery(|p: Post| p.blog_id)）
  ✅ CTE / Window 函数（WITH 子句 + ROW_NUMBER/RANK/SUM/LAG 等 10 种窗口函数）
  ✅ PostgreSQL HAVING 占位符 bug 修复（? → $N 共享 param_idx）
  ✅ PostgreSQL LIMIT/OFFSET 方言 bug 修复（gen.pagination() 委托）

v1.2+ 范围（规模扩展）:
  二级缓存（IQueryInterceptor + CachingProvider）
  读写分离自动路由（RoutingProvider）
  数据库分库分表（ShardingProvider）

v1.3+ 范围（生态集成）:
  GraphQL 集成
  示例项目扩展
```

---

# 已知限制（v1.1 使用者须知）

1. **`linq!` 需显式类型**：`|b: Blog|`，暂不支持省略（proc_macro 根本限制，已文档化）
2. **Lazy Loading 默认关闭**：需 `builder.use_lazy_loading(true)` 显式开启；开启后 `to_list()` 自动挂载延迟上下文
3. **拦截器只读**：`SaveChangesContext` 不含实体引用，无法在拦截器中改字段；软删除/时间戳需手动标记
4. **`from_row` 基于 `Vec<String>`**：大结果集性能与类型安全有限；Window 函数投影列被 `from_row` 忽略（仅读取实体字段）
5. **DbContext DI 为 Transient**：长生命周期场景需自行管理 scope
6. **CTE raw 模式 PostgreSQL 占位符**：`with_cte_internal()` 的预编译 SQL 使用 `?` 占位符，在 PostgreSQL 上不会转换为 `$N`。推荐使用 `linq!(with ...)` 语法糖（typed 模式），它在 `to_sql_with` 时用 provider 占位符编译，确保三库正确
7. **PostgreSQL Provider 默认 `NoTls`**：生产部署需自行启用 TLS（部署加固，非框架漏洞）

---

# 附录：测试清单（当前 272 个）

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
| `in_subquery_tests.rs` | 6 | IN / NOT IN 标量子查询（v1.1） |
| `lazy_loading_tests.rs` | 7 | Lazy Loading opt-in / HasMany / BelongsTo（v1.1） |
| `window_function_tests.rs` | 12 | Window 函数 SQL 生成 + 执行 + CTE raw 模式（v1.1） |
| `cte_syntax_tests.rs` | 13 | CTE 语法糖 SQL 生成 + 执行 + 参数顺序 + PG 多 CTE 占位符回归（v1.1） |
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

*下次审计建议触发条件：v1.2 启动（L2 缓存 / 读写分离 / 分库分表），或重大架构决策变更。*
