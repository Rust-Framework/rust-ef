# REF 框架生产级推进 — 续接计划（v1.3.0 → v1.4.0）

> 接续上一会话：P0-1（MySQL cell_to_string）、P0-2（MetadataCache poison 恢复）、P1-3（SQLite r2d2 池）、P1-6（PostgreSQL TLS 可配置）已完成。本计划聚焦剩余 2 个任务：Task B（PG/MySQL 测试对齐）+ Task C（版本/文档同步）。

---

## 当前状态分析（Phase 1 探索结论）

| 项 | 状态 | 证据位置 |
|----|:----:|---------|
| MySQL `cell_to_string` 类型分发 | ✅ | [mysql/connection.rs:20-68](file:///d:/GitCode/RF/rust-ef/crates/mysql/src/connection.rs#L20-L68) — bool→i64→u64→f64→NaiveDateTime→NaiveDate→Uuid→String→Vec\<u8\> |
| MetadataCache poison 恢复 | ✅ | [core/metadata_cache.rs:61-73](file:///d:/GitCode/RF/rust-ef/crates/core/src/metadata_cache.rs#L61-L73) — 清缓存重建，3 个单测 |
| SQLite r2d2 混合池 | ✅ | [sqlite/provider.rs:39-79](file:///d:/GitCode/RF/rust-ef/crates/sqlite/src/provider.rs#L39-L79) — `Pooled`/`Single` 枚举 + WAL customizer |
| PostgreSQL TLS 可配置 | ✅ | [postgres/provider.rs:18-80](file:///d:/GitCode/RF/rust-ef/crates/postgres/src/provider.rs#L18-L80) — `PgTlsMode` + `new_with_tls()`；[lib.rs:14](file:///d:/GitCode/RF/rust-ef/crates/postgres/src/lib.rs#L14) 导出；[di_extension.rs:45-61](file:///d:/GitCode/RF/rust-ef/crates/postgres/src/di_extension.rs#L45-L61) `use_postgres_with_tls`；[Cargo.toml:23-24](file:///d:/GitCode/RF/rust-ef/crates/postgres/Cargo.toml#L23-L24) `native-tls`+`postgres-native-tls 0.5` |
| 测试对齐共享 helper | ✅ | [core/tests/common/mod.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/common/mod.rs) — 8 个 `run_*` 函数已就绪 |
| PG/MySQL 测试文件 | ❌ | [postgres_crud_tests.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/postgres_crud_tests.rs)、[mysql_crud_tests.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/mysql_crud_tests.rs) 仍仅 1 个测试 |
| 工作区版本 | ❌ | 仍为 `1.3.0`（[Cargo.toml:16](file:///d:/GitCode/RF/rust-ef/Cargo.toml#L16)） |
| CHANGELOG | ❌ | 仅有 `[Unreleased]` 条目（metadata cache + rust-dix rename），未含 P0/P1 5 项变更，未升版 |
| PRODUCTION_READINESS_SPEC | ❌ | 仍标记 `v1.1.0`（[第 3 行](file:///d:/GitCode/RF/rust-ef/docs/PRODUCTION_READINESS_SPEC.md#L3)），未反映 v1.3 rust-dix 0.6、metadata cache、P0/P1 修复 |
| security.md TLS 章节 | ❌ | [第 86-88 行](file:///d:/GitCode/RF/rust-ef/docs/rust-ef/11-best-practices/security.md#L86-L88) 仍写 "PostgreSQL Provider 使用 NoTls" + "后续版本规划"，未提及已落地的 `PgTlsMode::Require` |

---

## 假设与决策

1. **范围**：仅完成已批准的 P0+P1 收尾，不引入 v1.2+ 路线图（L2 缓存/读写分离/分库分表）
2. **不重构** `Vec<String>` 类型擦除（`from_row`），沿用上一会话决策
3. **版本号**：1.3.0 → **1.4.0**（含新增 PG TLS API + SQLite 池化模式 + 跨 provider 测试对齐，符合 semver minor）
4. **CHANGELOG 结构**：将现有 `[Unreleased]` 条目保留为 v1.4.0 的一部分（metadata cache + rust-dix 0.6 已是 v1.4 范围），并追加 P0/P1 5 项
5. **SPEC 更新**：仅追加 v1.4 章节记录本轮迭代，不重写历史章节（v1.1/v1.0 内容保留）
6. **预存 clippy 警告**：`rust-ef-macros` 中 `large_enum_variant`/`needless_borrow` 为既有问题，超出本轮范围；C.5 回归使用 `--no-deps` 验证 rust-ef-* 自身，按 CLAUDE.md "Surgical Changes" 原则不处理
7. **测试环境**：本地无 PG/MySQL，新测试需在无 `RUST_EF_PG_URL`/`RUST_EF_MYSQL_URL` 时优雅跳过；CI 三库 matrix 会实际执行
8. **不创建 plan 之外的文档**：仅修改既有文件，遵循 CLAUDE.md 第 2/3 条

---

## Task B — P1-5 PG/MySQL 集成测试对齐

### B.1 修改 `crates/core/tests/postgres_crud_tests.rs`

**目标**：从 1 个测试扩展到 8 个测试，对齐 SQLite 的 9 个场景（scenario 5 update/delete 已在 `run_crud_lifecycle` 内）。

**实现**：保留现有 `pg_url()` 辅助函数与 `test_postgres_crud_lifecycle`。追加 7 个 `#[tokio::test]` 函数，每个调用 `common::run_*` helper。每个测试在 `RUST_EF_PG_URL` 未设置时优雅跳过（与现有模式一致）。

**新增测试列表**：
1. `test_postgres_filter_with_in_operator` → `common::run_filter_with_in_operator`
2. `test_postgres_limit_and_offset` → `common::run_limit_and_offset`
3. `test_postgres_count_and_any` → `common::run_count_and_any`
4. `test_postgres_aggregation_queries` → `common::run_aggregation_queries`
5. `test_postgres_empty_result_handling` → `common::run_empty_result_handling`
6. `test_postgres_ensure_created_and_deleted` → `common::run_ensure_created_and_deleted`
7. `test_postgres_has_data_seed` → `common::run_has_data_seed`

**import 更新**：将 `use common::run_crud_lifecycle;` 改为多行导入所有 8 个 helper（或保留单行 + 各测试内 `common::run_*` 全路径，按现有文件风格选择前者）。

**模板**（每个新测试统一模式）：
```rust
#[tokio::test]
async fn test_postgres_filter_with_in_operator() {
    let Some(url) = pg_url() else {
        eprintln!("skip test_postgres_filter_with_in_operator: RUST_EF_PG_URL not set");
        return;
    };
    let provider = match PostgresProvider::new(&url, 5) {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_postgres_filter_with_in_operator: {e}");
            return;
        }
    };
    common::run_filter_with_in_operator(provider, rust_ef::migration::MigrationDialect::Postgres)
        .await
        .expect("postgres filter with IN operator");
}
```

### B.2 修改 `crates/core/tests/mysql_crud_tests.rs`

**目标**：同 B.1，对 MySQL 应用相同 7 个测试。MySQL provider 构造是 `MySqlProvider::new(&url).await`（注意 `.await`，与 PG 不同）。

**新增测试列表**：
1. `test_mysql_filter_with_in_operator` → `common::run_filter_with_in_operator`
2. `test_mysql_limit_and_offset` → `common::run_limit_and_offset`
3. `test_mysql_count_and_any` → `common::run_count_and_any`
4. `test_mysql_aggregation_queries` → `common::run_aggregation_queries`
5. `test_mysql_empty_result_handling` → `common::run_empty_result_handling`
6. `test_mysql_ensure_created_and_deleted` → `common::run_ensure_created_and_deleted`
7. `test_mysql_has_data_seed` → `common::run_has_data_seed`

### B.3 验证

```bash
# 编译验证（无外部 DB 时应跳过所有 PG/MySQL 测试）
cargo test -p rust-ef --test postgres_crud_tests --features chrono,uuid,decimal --no-run
cargo test -p rust-ef --test mysql_crud_tests --features chrono,uuid,decimal --no-run

# 跳过验证（无 DB 环境下应全部 skip pass）
cargo test -p rust-ef --test postgres_crud_tests --features chrono,uuid,decimal
cargo test -p rust-ef --test mysql_crud_tests --features chrono,uuid,decimal
```

预期：编译通过，所有测试因环境变量未设置而跳过（输出 "skip ..."）。SQLite 测试不受影响。

---

## Task C — 版本/文档同步

### C.1 升级工作区版本 1.3.0 → 1.4.0

**修改文件**：
1. `Cargo.toml`（workspace 根）：[第 16 行](file:///d:/GitCode/RF/rust-ef/Cargo.toml#L16) `version = "1.3.0"` → `"1.4.0"`
2. `crates/core/Cargo.toml`：[第 12 行](file:///d:/GitCode/RF/rust-ef/crates/core/Cargo.toml#L12) `rust-ef-macros = { version = "1.3.0", ... }` → `"1.4.0"`
3. `crates/sqlite/Cargo.toml`：`rust-ef = { version = "1.3.0", ... }` → `"1.4.0"`
4. `crates/postgres/Cargo.toml`：[第 12 行](file:///d:/GitCode/RF/rust-ef/crates/postgres/Cargo.toml#L12) → `"1.4.0"`
5. `crates/mysql/Cargo.toml`：`rust-ef = { version = "1.3.0", ... }` → `"1.4.0"`
6. `crates/macros/Cargo.toml`：检查是否有版本硬编码
7. `crates/cli/Cargo.toml`：检查 `rust-ef` 依赖版本
8. `examples/*/Cargo.toml`：检查示例项目依赖版本

**验证**：`cargo build --workspace --all-features` 成功，无版本不一致警告。

### C.2 更新 CHANGELOG.md

**修改文件**：[CHANGELOG.md](file:///d:/GitCode/RF/rust-ef/CHANGELOG.md)

**操作**：
1. 将 `[Unreleased] — 2026-07-07 — Metadata cache + rust-dicore → rust-dix 0.6 rename` 标题改为 `[1.4.0] — 2026-07-08 — Production hardening (P0+P1) + metadata cache + rust-dix 0.6`
2. 在现有 metadata cache / rust-dix 内容**之前**追加 P0/P1 修复章节（按时间逻辑：生产硬化在前，架构迭代在后），共 5 个子章节：
   - `### Fixed — P0-1 MySQL cell_to_string 类型分发`（描述 bool/i64/u64/f64/NaiveDateTime/NaiveDate/Uuid/Bytes 按 sqlx 类型分发，修复非 String 列静默错误返回 "NULL" 的 bug）
   - `### Fixed — P0-2 MetadataCache poison 恢复`（描述 Mutex 中毒时清缓存重建而非 panic，3 个单测覆盖）
   - `### Added — P1-3 SQLite r2d2 连接池`（描述 `Pooled`/`Single` 混合模式 + WAL customizer + 5s busy timeout，文件模式默认 8 连接）
   - `### Added — P1-6 PostgreSQL TLS 可配置`（描述 `PgTlsMode` 枚举 + `new_with_tls()` + `use_postgres_with_tls()` + `postgres-native-tls 0.5` + 跨平台 native-tls）
   - `### Added — P1-5 PG/MySQL 集成测试对齐`（描述 7+7 个新测试对齐 SQLite 9 场景，共享 helper 在 common/mod.rs）
3. 在文件底部链接引用区追加：`[1.4.0]: https://gitcode.com/rf2026/rust-ef/releases/tag/v1.4.0`

### C.3 更新 PRODUCTION_READINESS_SPEC.md 到 v1.4

**修改文件**：[docs/PRODUCTION_READINESS_SPEC.md](file:///d:/GitCode/RF/rust-ef/docs/PRODUCTION_READINESS_SPEC.md)

**操作**：
1. 第 3 行版本号：`v1.1.0` → `v1.4.0`
2. 第 6 行当前阶段描述：追加 "v1.4 生产硬化迭代（P0 修复 + P1 加固）"
3. 在文件末尾（"下次审计建议触发条件" 之前）追加新章节 `## 3.10 v1.4 生产硬化迭代（2026-07-08）`，记录：
   - P0-1 / P0-2 / P1-3 / P1-5 / P1-6 五项变更（与 CHANGELOG 同步）
   - 新增 API 表面：`PgTlsMode`、`PostgresProvider::new_with_tls`、`DbContextOptionsBuilderExt::use_postgres_with_tls`、`SqliteProvider::new`（r2d2 池化）
   - 测试数量从 278 → 增量（14 个新 PG/MySQL 测试）
   - 验收准则表
4. 更新 "已知限制" 第 7 条：`PostgreSQL Provider 默认 NoTls` → 改为说明可通过 `PgTlsMode::Require` 启用
5. 更新 "实现优先级" 区块，将 P0/P1 标记为已完成

### C.4 更新 security.md TLS 章节

**修改文件**：[docs/rust-ef/11-best-practices/security.md](file:///d:/GitCode/RF/rust-ef/docs/rust-ef/11-best-practices/security.md)

**操作**：替换第 84-88 行 "TLS / 传输加密" 子章节，从 "Provider 使用 NoTls + 后续版本规划" 改为：

```markdown
### TLS / 传输加密

PostgreSQL Provider 自 v1.4 起支持可配置 TLS：

```rust
use rust_ef_postgres::{PostgresProvider, PgTlsMode};

// 方式 1: 显式 NoTls（向后兼容 v1.3，仅用于本地开发）
let provider = PostgresProvider::new(&url, 5)?;

// 方式 2: 强制 TLS（生产推荐）
let connector = native_tls::TlsConnector::builder()
    .add_root_certificate(/* 加载 CA 证书 */)
    .build()?;
let provider = PostgresProvider::new_with_tls(
    &url, 5, PgTlsMode::Require(connector)
)?;
```

`PgTlsMode::Require` 使用平台原生 TLS 实现（Windows SChannel / Linux OpenSSL / macOS Secure Transport）。TLS 类型在 `deadpool_postgres::Manager` 内部通过 `Box<dyn Connect>` 擦除，因此 `Pool` 与 `PostgresConnection` 保持非泛型 API。

> **MySQL/SQLite**：MySQL 经 sqlx 已支持 `mysql://...?tls=true` 连接串参数；SQLite 为进程内数据库，无需 TLS。生产部署若跨不可信网络，仍建议结合 SSH 隧道或 VPN。
```

同时更新第 173 行 "已知限制：PostgreSQL NoTls" → 移除该限制条目（或改为 "已支持"）。

### C.5 全量回归验证

**验证步骤**（按顺序执行，任一失败立即停止）：

```bash
# 1. 格式检查
cargo fmt --all -- --check

# 2. Clippy（rust-ef-* 自身，跳过 macros 既有警告）
cargo clippy --workspace --all-features --no-deps -- -D warnings

# 3. 全量测试（无外部 DB 时 PG/MySQL 测试 skip）
cargo test --workspace --all-features --no-fail-fast

# 4. 基准编译验证
cargo bench --workspace --no-run

# 5. SQLite 单库验证（确保 P1-3 r2d2 改动未破坏现有测试）
cargo test -p rust-ef --features chrono,uuid,decimal --test sqlite_crud_tests
```

**预期结果**：
- fmt 通过
- clippy 零新增警告（既有 macros 警告用 `--no-deps` 隔离）
- 全部测试通过或 skip（PG/MySQL 因无 DB 跳过）
- 基准可编译
- SQLite 9 场景全绿

---

## 实施顺序

1. **B.1 + B.2 并行**（独立文件，无依赖）→ B.3 编译/跳过验证
2. **C.1 版本升级** → `cargo build --workspace --all-features` 验证
3. **C.2 CHANGELOG** → 人工 review 内容完整性
4. **C.3 SPEC** → 人工 review
5. **C.4 security.md** → 人工 review
6. **C.5 全量回归** → 5 个验证命令

---

## 风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|:----:|:----:|------|
| common/mod.rs 的 `has_data` helper 在 PG/MySQL 上行为不一致（种子数据方言差异） | 中 | 中 | helper 内部使用 `DbContext::ensure_created` + `save_changes`，已抽象方言；若 CI 失败，按实际错误追加方言特定断言 |
| PG/MySQL 测试因 `reset_schema` 中 `ensure_deleted`/`ensure_created` 跨测试串扰 | 低 | 中 | 每个 helper 独立调用 `reset_schema`，且 `ensure_deleted` 失败被忽略（`let _ =`） |
| 版本升级遗漏某 crate 的 inter-dep | 中 | 低 | C.1 已列出所有需检查的 Cargo.toml；`cargo build --workspace` 会立即报错 |
| security.md 代码示例编译失败 | 低 | 低 | 示例为说明性片段，不进 doctest；C.5 不覆盖文档示例 |
| 既有 macros clippy 警告阻塞 C.5 | 高 | 低 | 使用 `--no-deps` 隔离；既定不处理（超出范围） |

---

## 完成标准

- [ ] B.1 PG 测试文件 8 个测试就位
- [ ] B.2 MySQL 测试文件 8 个测试就位
- [ ] B.3 编译 + 跳过验证通过
- [ ] C.1 工作区版本统一 1.4.0
- [ ] C.2 CHANGELOG v1.4.0 条目完整（5 个 P0/P1 子章节）
- [ ] C.3 SPEC 升至 v1.4，追加 3.10 章节
- [ ] C.4 security.md TLS 章节反映 `PgTlsMode::Require`
- [ ] C.5 5 项回归命令全绿（PG/MySQL 测试可 skip）
