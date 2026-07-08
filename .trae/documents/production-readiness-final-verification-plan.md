# REF 框架 v1.4 生产硬化迭代 — 收尾计划（C.5 回归验证）

> 续接上一会话：Task B（PG/MySQL 测试对齐）+ Task C.1–C.4（版本/CHANGELOG/SPEC/security.md）全部完成。本计划仅聚焦 **Task C.5 全量回归验证的剩余步骤 3–5**。

---

## 当前状态分析（Phase 1 探索结论）

### 已完成（无需重复）

| 项 | 状态 | 证据 |
|----|:----:|------|
| PG 测试文件 8 个测试 | ✅ | [postgres_crud_tests.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/postgres_crud_tests.rs) — `test_postgres_crud_lifecycle` + 7 个新测试（filter_with_in / limit_and_offset / count_and_any / aggregation / empty_result / ensure_created_and_deleted / has_data_seed） |
| MySQL 测试文件 8 个测试 | ✅ | [mysql_crud_tests.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/mysql_crud_tests.rs) — 同 PG 8 项对齐 SQLite 9 场景 |
| common/mod.rs 共享 helper | ✅ | 8 个 `run_*` 函数 + `TestItem::COLUMN_NAME/COLUMN_VALUE` 常量已就位 |
| 工作区版本 1.4.0 | ✅ | [Cargo.toml:16](file:///d:/GitCode/RF/rust-ef/Cargo.toml#L16) `version = "1.4.0"`；core/sqlite/postgres/mysql/cli 的 inter-dep 全部 1.4.0 |
| CHANGELOG v1.4.0 | ✅ | [CHANGELOG.md:12](file:///d:/GitCode/RF/rust-ef/CHANGELOG.md#L12) 含 5 个 P0/P1 子章节 + 链接引用 |
| SPEC v1.4 | ✅ | [PRODUCTION_READINESS_SPEC.md:3](file:///d:/GitCode/RF/rust-ef/docs/PRODUCTION_READINESS_SPEC.md#L3) 版本升至 v1.4.0 |
| security.md TLS 章节 | ✅ | [security.md:84-105](file:///d:/GitCode/RF/rust-ef/docs/rust-ef/11-best-practices/security.md#L84-L105) 含 `PgTlsMode::Require` 代码示例 |
| C.5 步骤 1：fmt | ✅ | `cargo fmt --all -- --check` 通过（已先 `cargo fmt --all` 自动修复） |
| C.5 步骤 2：clippy | ✅ | 6 个既有警告（3 macros + 3 core），本会话零新增 |

### 待完成

| 步骤 | 状态 | 说明 |
|------|:----:|------|
| C.5 步骤 3：全量测试 | ⚠️ 退出码 101 | `cargo test --workspace --all-features --no-fail-fast` 因 `Select-String` 过滤截断输出，**未识别具体失败 target**；多数测试套件显示通过 |
| C.5 步骤 4：基准编译 | ❌ 未执行 | `cargo bench --workspace --no-run` |
| C.5 步骤 5：SQLite 单库 | ❌ 未执行（B.3 已验证） | `cargo test -p rust-ef --features chrono,uuid,decimal --test sqlite_crud_tests` |

---

## 假设与决策

1. **范围**：仅完成 C.5 剩余 3 个验证步骤 + 必要修复，不引入新功能/新文档/新测试
2. **失败处理原则**：
   - 若步骤 3 的失败是**本会话引入的**（如 PG/MySQL 测试编译错误、common/mod.rs 修改副作用）→ 立即修复
   - 若失败是**既有问题**（与本会话无关，如 macros clippy 警告、pre-existing 测试问题）→ 记录在最终响应中，不处理
3. **环境**：本机无 PG/MySQL 服务，PG/MySQL 测试应通过 skip 机制跳过；为避免 `CI=true` 触发默认连接串，验证命令前置 `$env:CI=''`（PowerShell 语法）
4. **不重做已完成项**：依据 CLAUDE.md "Surgical Changes" 原则，不重跑 fmt / clippy / 已通过的部分测试套件
5. **完成即汇报**：C.5 全绿后立即给出最终响应，不调用 NotifyUser（已在 Plan Mode 末尾用过一次）

---

## 实施步骤

### 步骤 1：诊断 C.5 步骤 3 的失败 target

**目标**：识别 `cargo test --workspace --all-features --no-fail-fast` 退出码 101 的根因。

**实现**：

```powershell
$env:CI=''; cargo test --workspace --all-features --no-fail-fast 2>&1 | Tee-Object -FilePath .\.trae\documents\c5-step3-full-test.log
```

> 用 `Tee-Object` 完整保存输出到日志文件，便于事后审计；不使用 `Select-String` 过滤，避免遗漏关键错误信息。

**判断分支**：

- **A. 编译失败**（某个 test target 无法编译）→ 查看日志中的 `error[E...]` 行，定位文件 + 行号 → 修复 → 重新执行步骤 1
- **B. 测试失败**（某个测试运行时失败）→ 查看日志中的 `FAILED` / `test ... failed` 行 → 判断：
  - 若是本会话引入的（PG/MySQL 新测试或 common/mod.rs 修改）→ 修复 → 重新执行步骤 1
  - 若是既有测试（如 chrono/uuid/decimal feature gate 相关、与 P0/P1 无关）→ 记录到最终响应，不处理
- **C. 全部通过或仅 PG/MySQL skip**（之前误判为失败）→ 标记步骤 3 完成，进入步骤 2

**预期成功标准**：日志末尾出现 `test result: ok. N passed; 0 failed; M ignored` 或仅 PG/MySQL 测试因无 DB 跳过；无 `error:` 行；exit code 0。

---

### 步骤 2：完成 C.5 步骤 4 — 基准编译验证

**目标**：确认 Criterion 基准 (`crates/core/benches/*.rs`) 在 `--all-features` 下可编译。

**实现**：

```powershell
cargo bench --workspace --no-run 2>&1 | Tee-Object -FilePath .\.trae\documents\c5-step4-bench.log
```

**判断分支**：

- **A. 编译通过**（exit 0）→ 标记步骤 4 完成，进入步骤 3
- **B. 编译失败** → 查看日志中的 `error[E...]` → 判断：
  - 若与本会话相关（如 `bench_insert.rs` 引用了被本会话修改的 API）→ 修复
  - 若与本会话无关（pre-existing 基准问题）→ 记录，不处理，继续步骤 3

**预期成功标准**：`Compiling` + `Finished` 输出，无 `error:` 行。

---

### 步骤 3：完成 C.5 步骤 5 — SQLite 单库回归

**目标**：最终确认 P1-3（SQLite r2d2 池化）改动未破坏 SQLite 9 场景。

**实现**：

```powershell
$env:CI=''; cargo test -p rust-ef --features chrono,uuid,decimal --test sqlite_crud_tests 2>&1 | Tee-Object -FilePath .\.trae\documents\c5-step5-sqlite.log
```

**判断分支**：

- **A. 9 个测试全绿**（exit 0）→ 标记步骤 5 完成，进入步骤 4
- **B. 任意测试失败** → 查看日志 → 判断：
  - 若与 r2d2 池化相关（如 `SQLITE_BUSY`、连接获取失败）→ 修复 `SqliteProvider` 或测试隔离逻辑
  - 若与 r2d2 无关（既有 SQLite 问题）→ 记录，不处理

**预期成功标准**：`test result: ok. 9 passed; 0 failed`；exit 0。

> 注：B.3 已在 PG/MySQL 验证时确认 SQLite 9 场景通过，本步骤是最终回归确认，预期 PASS。

---

### 步骤 4：标记 Task #24 完成 + 最终响应

**目标**：完成全部 C.5 验证后，更新任务状态并向用户汇报。

**实现**：
1. 通过 `TaskUpdate` 将 Task #24 (C.5) 标记为 `completed`，summary 字段记录 5 步验证结果
2. 直接向用户返回最终响应（不调用 NotifyUser — Plan Mode 末尾已用过）

**最终响应内容**（结构化中文汇报）：

```markdown
# REF v1.4.0 生产硬化迭代 — 全部完成

## 本轮交付（P0+P1 共 5 项）

| 优先级 | 项目 | 状态 | 关键文件 |
|:------:|------|:----:|---------|
| P0-1 | MySQL cell_to_string 类型分发 | ✅ | crates/mysql/src/connection.rs |
| P0-2 | MetadataCache poison 恢复 | ✅ | crates/core/src/metadata_cache.rs |
| P1-3 | SQLite r2d2 连接池 | ✅ | crates/sqlite/src/provider.rs |
| P1-5 | PG/MySQL 测试对齐 SQLite 9 场景 | ✅ | crates/core/tests/{postgres,mysql}_crud_tests.rs + common/mod.rs |
| P1-6 | PostgreSQL TLS 可配置 | ✅ | crates/postgres/src/{provider,lib,di_extension}.rs |

## 版本/文档同步

- 工作区版本：1.3.0 → 1.4.0（6 个 Cargo.toml 统一）
- CHANGELOG：v1.4.0 条目（5 个 P0/P1 子章节）
- SPEC：v1.4.0（追加 3.10 章节 + 更新已知限制 + 实现优先级）
- security.md：TLS 章节替换为 PgTlsMode::Require 文档

## 回归验证结果

- [✅/⚠️] 步骤 1 fmt：通过（auto-fix 后 --check 全绿）
- [✅/⚠️] 步骤 2 clippy：零新增警告（6 个既有警告记录在案）
- [✅/⚠️] 步骤 3 全量测试：<具体结果>
- [✅/⚠️] 步骤 4 基准编译：<具体结果>
- [✅/⚠️] 步骤 5 SQLite 9 场景：<具体结果>

## 新增 API 表面

- `rust_ef_postgres::PgTlsMode::{Disable, Require}`
- `PostgresProvider::new_with_tls(url, pool_size, tls)`
- `DbContextOptionsBuilderExt::use_postgres_with_tls(...)`
- `SqliteProvider::new(path)` 默认 r2d2 池化 + WAL + 5s busy_timeout
- `SqliteProvider::new_in_memory()` 保留 `:memory:` 单连接语义

## 已知遗留（不在本轮范围）

- `rust-ef-macros` 3 个 clippy 警告（large_enum_variant + 2x needless_borrow）— 既有问题
- `crates/core` 3 个 clippy 警告（unused_import + 2x doc_lazy_continuation）— 既有问题
- <步骤 3 若发现既有测试问题，在此列出>

## 测试数量

- v1.3.0：278 个测试
- v1.4.0：278 + 14（PG 7 + MySQL 7）+ 3（MetadataCache 单测）= ~295 个测试
```

---

## 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|:----:|:----:|------|
| 步骤 3 失败的 target 是本会话引入的问题（编译错误或测试逻辑 bug） | 中 | 中 | 完整日志保存到 `.trae/documents/c5-step3-full-test.log`，按 `error[E...]` 或 `FAILED` 行定位 → 修复 → 重跑 |
| 步骤 3 失败的 target 是既有问题被本次 `--all-features` 暴露 | 低 | 低 | 记录到"已知遗留"，不处理（超出本轮范围） |
| 步骤 4 bench 编译失败因 API 改动 | 低 | 低 | bench 文件未在本会话修改，预期通过；若失败按错误信息判断归属 |
| 步骤 5 SQLite 测试因 r2d2 行为差异失败 | 低 | 中 | B.3 已验证通过；若失败优先排查 WAL/busy_timeout 与 `:memory:` 模式混淆 |
| `Tee-Object` 在 PowerShell 中行为异常 | 低 | 低 | 改用 `>` 重定向 + 后续 Read 工具读取日志文件 |

---

## 完成标准

- [ ] 步骤 1：C.5 步骤 3 失败 target 已识别并修复（或确认非本会话问题）
- [ ] 步骤 2：C.5 步骤 4 `cargo bench --workspace --no-run` 编译通过
- [ ] 步骤 3：C.5 步骤 5 SQLite 9 场景全绿
- [ ] 步骤 4：Task #24 标记完成 + 最终响应已发送

---

## 不在本计划范围

- v1.5+ 路线图（L2 缓存、读写分离、分库分表、GraphQL）
- `Vec<String>` 类型擦除重构
- `rust-ef-macros` 既有 clippy 警告修复
- 新增文档/示例/功能
