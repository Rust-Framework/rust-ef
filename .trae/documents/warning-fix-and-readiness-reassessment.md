# REF v1.4.1 编译警告修复 + 生产就绪重新评估

## Context

v1.4.0 生产硬化迭代完成后，全量回归测试通过（320 个测试全绿），但仍存在 6 个编译警告（3 macros + 3 core）。用户要求：
1. **解决编译警告/错误问题** — 让代码库达到零警告状态
2. **重新评估生产就绪状态** — 从性能、并发、安全、易用性、架构优越性等维度全面复审

本计划完成上述两项任务，输出为零警告代码库 + 更新后的生产就绪评估文档。

---

## Part A：修复 6 个编译警告

所有警告已通过 Phase 1 探索精确定位，修复策略经 Plan agent 验证。均为外科手术式修改，不改变运行时语义。

### A.1 `crates/macros/src/linq/ast.rs:56` — large_enum_variant

**问题**：`pub(crate) enum LinqClause` 有 28+ 变体，最大变体携带 `Expr`/`Vec<Expr>`/`(Expr, Expr)`，触发 large_enum_variant 警告。

**修复**：在 enum 定义上方添加 `#[allow(clippy::large_enum_variant)]`。

**理由**：这是 proc_macro 编译期 AST，永不进入运行时实例化。Box 包装每个 `Expr` 字段会侵入所有 match 臂，增加宏展开复杂度且无运行时性能收益。

### A.2-3 `crates/macros/src/linq/expand.rs:237, 238` — needless_borrow

**问题**：
```rust
let (fk_expr, pk_expr) = link.as_ref().expect("...");
let fk_col = extract_field_name_only(&fk_expr)?;  // &&Expr，多余 &
let pk_col = extract_field_name_only(&pk_expr)?;  // &&Expr，多余 &
```

`link.as_ref()` 返回 `&(Expr, Expr)`，解构后 `fk_expr: &Expr`。`extract_field_name_only` 签名是 `fn(expr: &Expr)`（expand.rs:399），所以 `&fk_expr` 是 `&&Expr`，clippy 正确。

**修复**：移除 `&`，改为 `extract_field_name_only(fk_expr)?` 和 `extract_field_name_only(pk_expr)?`。

### A.4 `crates/core/src/query/select.rs:15` + 161-165 — unused_import（双重删除）

**问题**：select.rs 有两处导入 `PortablePlaceholderGenerator`：
- Line 15：`use super::compile::PortablePlaceholderGenerator;`（顶层，unused → 警告）
- Line 161-165：`#[allow(unused_imports)] use ... as _PortablePlaceholderGenerator;`（别名，也 unused，用 allow 压制）

**已验证**：`PortablePlaceholderGenerator` 是 `pub(crate) struct`（compile.rs:23），非 trait。它在 `state.rs:331` 内部构造（`self.to_sql_with(&PortablePlaceholderGenerator)`）。select.rs 从未直接引用该类型——`to_sql()` 调用 `self.state.to_sql()`，trait 方法已在 state.rs 内解析。

**修复**：删除 line 15 + 删除 line 161-165 整个块（注释 + `#[allow]` + 别名导入）。

### A.5-6 `crates/core/src/model_builder.rs:68-70` — doc_lazy_continuation

**问题**：line 68 `/// + re-running...` 的 `+` 被 markdown 解释为列表项起始，line 69-70 成为"惰性延续"（无缩进）触发警告。

**修复**：将 line 68 的 `/// + re-running` 改为 `/// and re-running`（移除 `+` 避免列表项解释）。

---

## Part B：生产就绪重新评估（8 维度）

经 Plan agent 验证，原 5 维度需补充 3 个：可观测性、错误处理、向后兼容/Semver。评估以新章节追加到现有 `docs/PRODUCTION_READINESS_SPEC.md`，保持单一信息源。

### B.1 新增章节：`## 4. 生产就绪维度重新评估（2026-07-08）`

**位置**：`docs/PRODUCTION_READINESS_SPEC.md` 末尾（在"下次审计建议触发条件"之前，若有）

**结构**：每个维度包含「现状」「优势」「风险/改进」「就绪评级」

#### 维度 1：性能 (Performance)
- **现状**：MetadataCache 进程级缓存（Mutex<HashMap>，Arc clone 后无锁）、SQLite r2d2 池（8 连接 + WAL + 5s busy_timeout）、PG/MySQL deadpool、Criterion 基准（insert/query/include）
- **优势**：元数据一次构建多次复用；连接池避免握手开销；WAL 允许读写并发
- **风险**：`save_changes()` 遍历所有跟踪实体（O(n)）；MetadataCache Mutex 在首次构建时阻塞同 key 后续请求
- **评级**：✅ 就绪（中小规模生产）；⚠️ 大规模需基准验证

#### 维度 2：并发 (Concurrency)
- **现状**：DbContext Scoped 生命周期（每请求独立）、owned DbContext 支持 `&mut self`（无锁）、MetadataCache poison 恢复、事务支持（begin/commit/rollback + savepoint + isolation level）
- **优势**：owned resolution 消除 Arc<Mutex>；Scoped 隔离跟踪状态；WAL + busy_timeout 减少 SQLITE_BUSY
- **风险**：MetadataCache 用 Mutex（非 RwLock），高频读取场景可能争用；SQLite 写锁全局串行
- **评级**：✅ 就绪

#### 维度 3：安全 (Security)
- **现状**：全 SQL 参数化（DbValue）、标识符来自编译期实体元数据、`PgTlsMode::Require`（v1.4）、迁移引擎结构化、查询过滤器（多租户隔离）、乐观并发 token
- **优势**：零字符串拼接 SQL；编译期类型安全；TLS 可配置
- **风险**：无 raw SQL 逃生舱（复杂查询场景受限）；MySQL TLS 依赖连接串参数（非显式 API）
- **评级**：✅ 就绪

#### 维度 4：易用性 (Usability)
- **现状**：`linq!` 宏 DSL、`#[derive(EntityType)]` 12+ 属性、DI 集成（rust-dix 0.6）、3 个示例（blog/soft_delete/audit）、mdBook 文档、CLI 工具
- **优势**：类型安全 DSL 消除字符串 API；自动实体发现；DI 集成降低样板
- **风险**：`linq!` 宏有学习曲线；无 scaffold 从现有 DB 反向生成（CLI 有但可能有限）；文档示例部分未进 doctest
- **评级**：✅ 就绪

#### 维度 5：架构优越性 (Architecture)
- **现状**：crate 分层（core/sqlite/postgres/mysql/macros/cli）、provider 抽象（IDatabaseProvider/ISqlGenerator/IAsyncConnection）、模块化 query/migration/entity、SaveChanges 拦截器、软删除/审计/多租户、type-map DbContext
- **优势**：清晰职责分离；provider 可插拔；拦截器管道可扩展；metadata cache 透明
- **风险**：db_context.rs 仍较大（需评估行数）；interceptor.rs 有编码损坏（pre-existing，非本范围）
- **评级**：✅ 就绪

#### 维度 6：可观测性 (Observability) — 新增
- **现状**：EFError 错误传播；无 tracing/log 集成；无慢查询日志；无连接池指标暴露
- **优势**：错误信息含上下文（SQL/参数）
- **风险**：生产环境无法观测 ORM 层耗时；连接池饱和无告警；事务失败无结构化日志
- **评级**：⚠️ 未就绪（建议 v1.5 引入 `tracing` instrument）

#### 维度 7：错误处理 (Error Handling) — 新增
- **现状**：`EFError` 枚举（Connection/Migration/Concurrency/Transaction/Validation 等）、`EFResult<T>` 统一返回、`?` 传播
- **优势**：错误分类清晰；不 panic（MetadataCache poison 也恢复）；并发冲突显式返回
- **风险**：错误信息可能泄露内部细节（如 SQL/表名）给上层；无错误码体系；部分错误可能过于宽泛
- **评级**：✅ 基本就绪（生产可用，但建议 v1.5 细化错误分类）

#### 维度 8：向后兼容 / Semver — 新增
- **现状**：v1.0 GA 后 v1.1/v1.3/v1.4 均有 breaking change（`#[entity_config]` → `#[entity]`、rust-dix 0.6 API、`begin_transaction` 签名）；CHANGELOG 详细记录迁移步骤
- **优势**：CHANGELOG 迁移指南完整；版本号语义清晰
- **风险**：minor 版本含 breaking change（违反 SemVer 严格解读）；无 deprecation 期；用户升级需改代码
- **评级**：⚠️ 部分就绪（建议 v1.5+ 严格 SemVer，breaking change 预留 deprecation）

### B.2 总体就绪结论

在 SPEC 新章节末尾给出总体结论：
- **生产就绪维度**：5/8 完全就绪（性能/并发/安全/易用性/架构/错误处理），2/8 基本就绪，2/8 未就绪（可观测性/Semver）
- **推荐场景**：中小规模 Web 服务、内部工具、多租户 SaaS（含 PG TLS）
- **暂不推荐**：需要深度可观测性的大规模生产、对 API 稳定性极敏感的场景
- **v1.5 优先项**：tracing 集成、SemVer 严格化、MySQL TLS 显式 API

### B.3 更新 SPEC 顶部元信息

- 版本号：`v1.4.0` → `v1.4.1`（含警告修复 + 重新评估）
- 当前阶段：追加"v1.4.1 零警告 + 8 维度生产就绪复审"

---

## 实施步骤

1. **A.1** 在 `crates/macros/src/linq/ast.rs` 第 56 行 `pub(crate) enum LinqClause` 上方添加 `#[allow(clippy::large_enum_variant)]`
2. **A.2-3** 在 `crates/macros/src/linq/expand.rs` 移除 line 237、238 的 `&`（`&fk_expr` → `fk_expr`，`&pk_expr` → `pk_expr`）
3. **A.4** 在 `crates/core/src/query/select.rs` 删除 line 15 + 删除 line 161-165 整个块
4. **A.5-6** 在 `crates/core/src/model_builder.rs` 将 line 68 `/// + re-running` 改为 `/// and re-running`
5. **B.1-B.2** 在 `docs/PRODUCTION_READINESS_SPEC.md` 末尾追加 `## 4. 生产就绪维度重新评估（2026-07-08）` 章节（8 维度评估 + 总体结论）
6. **B.3** 更新 SPEC 顶部版本号和当前阶段描述

---

## 验证

```powershell
# 1. 零警告验证（应无任何 warning 输出）
$env:CI=''; cargo clippy --workspace --all-features --no-deps -- -D warnings 2>&1 | Select-String "warning:|error"

# 2. 全量测试通过
$env:CI=''; cargo test --workspace --all-features --no-fail-fast

# 3. fmt 检查
cargo fmt --all -- --check

# 4. 基准编译
cargo bench --workspace --no-run
```

**预期**：
- clippy 零警告（`-D warnings` 通过）
- 全量测试全绿（320 个测试）
- fmt 通过
- 基准编译通过

---

## 不在本计划范围

- 可观测性实施（tracing 集成）— v1.5
- SemVer 严格化策略 — v1.5
- MySQL TLS 显式 API — v1.5
- interceptor.rs 编码损坏修复（pre-existing，非本会话引入）
- db_context.rs 重构（pre-existing，需独立评估）
- v1.5+ 路线图（L2 缓存、读写分离、分库分表）
