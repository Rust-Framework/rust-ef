# Changelog

All notable changes to **rust-ef** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> 注：IDbContext trait 已在后续版本中移除，本文档中相关内容仅作历史记录。

---

## [1.6.0] — 2026-07-09 — Production Hardening（4 P0 + 12 P1）

### Summary

v1.6.0 是生产硬化迭代，修复 4 个 P0 阻塞项和 12 个 P1 改进项。涵盖连接管理、安全
默认值、类型系统物化、性能批量化、完整性 API 五大领域。**含破坏性变更**——详见
`docs/v1.5-semver-migration-guide.md` 的 v1.6 章节。

### P0 — 阻塞项修复

#### M1.1 连接池单例化

`DbContextOptions.provider_cache` 进程级缓存 provider（含连接池），所有同源
`DbContext` 复用同一连接池。避免每请求重建池导致的连接泄漏。

#### M1.2 Debug 脱敏

`DbContextOptions` 的 `Debug` 实现脱敏连接串中的凭据（URL 形式 `user:pass@host`
和 key=value 形式 `Password=...`），防止日志泄漏。

#### M1.3 BoolExpr::raw 收窄

`BoolExpr::raw()` 从 `pub` 收窄为 `pub(crate)`，外部代码无法构造 `RawSql` 变体，
从类型层面消除 SQL 注入风险。

#### M1.4 TLS 安全优先

PostgreSQL TLS 默认从 `Disable` 改为 `Require`；MySQL TLS 默认从 `Disabled` 改为
`Required`。`Disable`/`Disabled` 仍可通过显式 `*_with_tls` API 选用（逃生舱）。

### P1 — 改进项

#### M2 De-String 物化管道

`IFromRow::from_row` 签名从 `&[String]` 改为 `&[DbValue]`，消除全链路 String 物化。
新增 `TryFrom<DbValue>` for `i8/u32/u64`，provider 层 `cell_to_db_value` 替代
`cell_to_string`。**破坏性变更**。

#### M3 P1 稳定性

- 字符串 PK include 测试覆盖
- Layer 8（consumer 层）全量验证

#### M4 P1 性能

- C1: 批量 UPDATE 用 `CASE pk WHEN ? THEN ?` 减少 N 次往返为 1 次
- C2: 批量 INCLUDE 用 UNION 策略减少 N 次查询为 1 次
- C3: 连接池 acquire 指标 tracing
- C4: 批量 INSERT 参数布局优化

#### M5 P1 完整性

**D1: 属性级变更追踪 + 部分 UPDATE**

`detect_changes` 收集所有变更字段名到 `modified_properties: Vec<String>`。
`execute_updates` 仅 SET 脏列（批量 CASE WHEN 用并集，逐行用各实体自己的列表）。
`modified_properties` 为空时回退到全列 SET（向后兼容）。并发令牌安全：令牌递增后
出现在 `modified_properties` 中并被 SET，WHERE 始终检查原始令牌值。

**D2: 批量 INSERT 主键回填**

- PostgreSQL: `INSERT ... RETURNING *` 直接读取生成的主键
- SQLite: `SELECT last_insert_rowid()`（返回最后 rowid，计算 `last-N+1..last`）
- MySQL: `SELECT LAST_INSERT_ID()`（返回首个 ID，计算 `first..first+N-1`）

新增 `ISqlGenerator::supports_returning()` / `last_insert_id_sql()` /
`last_insert_id_returns_first()` 默认方法。新增 `IGetKeyValues::set_auto_increment_key`
trait 方法（默认 no-op，宏为 `#[auto_increment] #[primary_key]` 字段生成覆盖）。

`save_changes` 后行为变更：Added/Modified → Unchanged（带刷新快照），Deleted 移除。
`accept_all_changes` 替代 `clear_entries`，保留已保存实体（含回填主键）可查询。

**D3: Upsert API**

新增 `DbSet::upsert(&mut self, entity: T)` — 标记 Added + `is_upsert: true`。
`save_changes` 将 upsert 条目路由到 `execute_upserts`，生成：

- SQLite/PostgreSQL: `INSERT ... ON CONFLICT(pk) DO UPDATE SET col = EXCLUDED.col`
- MySQL: `INSERT ... ON DUPLICATE KEY UPDATE col = VALUES(col)`

冲突目标为主键列。INSERT 包含所有列（含自增主键）以确保用户提供的键值参与冲突检查。
不做主键回填——upsert 调用者需自行管理键。

新增 `ISqlGenerator::upsert_batch()` 默认方法，三 provider 实现。

**D4: Raw SQL → 实体映射**

新增 `DbContext::sql_query<T: IFromRow + IEntityType>(&self, sql: &str, params: &[DbValue])` —
复杂查询（多表 JOIN、CTE、窗口函数）的逃生舱。复用 M2 后的 `IFromRow`（已支持
`&[DbValue]`），无需新 trait。

### 破坏性变更摘要

| 变更 | 影响 | 迁移方式 |
|------|------|----------|
| `IFromRow::from_row` 签名 `&[String]` → `&[DbValue]` | 自定义实体需重写 `from_row` | 用 `TryFrom<DbValue>` |
| `ParseFromDb` trait 移除 | 使用该 trait 的代码需迁移 | 改用 `TryFrom<DbValue>` |
| TLS 默认值变更（PG `Require`、MySQL `Required`） | 明文连接需显式声明 | 用 `*_with_tls(Disable)` |
| `BoolExpr::raw` 收窄为 `pub(crate)` | 外部无法构造 `RawSql` | 用参数化查询 |
| `save_changes` 后保留实体（不 clear） | 依赖 clear 行为的代码需手动 `clear_entries` | 调用 `db_set.clear_entries()` |
| `DbValue` 新增 `TryFrom` for i8/u32/u64 | 无破坏（additive） | — |

---

## [1.5.0] — 2026-07-08 — tracing 集成 + SemVer 严格化 + MySQL TLS 显式 API

### Added — tracing 集成（慢查询 + 连接池指标）

新增 `tracing` 可选 feature（core + sqlite / postgres / mysql 三个 provider），通过
`tracing` crate 发射结构化事件。feature 关闭时全部 Guard 为 ZST no-op，编译期消除，
零运行时开销。

**3 个 Guard 类型**（`crates/core/src/observability.rs`）：

- `QueryGuard` — 包裹单次 `query` / `execute` 调用；启动时发 `DEBUG` 事件，完成时发
  `DEBUG`（正常）或 `WARN`（超过慢查询阈值）。target：`rust_ef::query`
- `PoolAcquireGuard` — 包裹连接池 acquire；完成时发 `INFO` 事件含 acquire 耗时。
  target：`rust_ef::pool`
- `SaveChangesGuard` — 包裹 `save_changes` 调用；完成时发 `INFO` 事件含总耗时。
  target：`rust_ef::save_changes`

**配置 API**：

```rust
use std::time::Duration;
use rust_ef::DbContextOptionsBuilder;

let mut options = DbContextOptionsBuilder::new();
options
    .use_sqlite("app.db")
    .slow_query_threshold(Duration::from_millis(500)); // 超过 500ms 的查询发 WARN
```

**trait 扩展**（cfg-gated 默认方法，非 breaking）：

- `IAsyncConnection::set_slow_query_threshold(&mut self, Duration)` — 连接级阈值注入
- `IDatabaseProvider::set_slow_query_threshold(&self, Duration)` — provider 级阈值存储

**实现文件**：

- `crates/core/src/observability.rs` — 3 个 Guard 的 cfg-gated 双实现
- `crates/core/src/db_context.rs` — `DbContextOptions.slow_query_threshold` 字段 +
  builder 方法 + `create_provider` 注入 + `save_changes` 加 `SaveChangesGuard`
- `crates/core/src/provider.rs` — 两个 trait 的 cfg-gated 默认方法
- `crates/{sqlite,postgres,mysql}/src/provider.rs` — `AtomicU64` 阈值存储 +
  `PoolAcquireGuard` 在 `get_connection` + `set_slow_query_threshold` impl
- `crates/{sqlite,postgres,mysql}/src/connection.rs` — 阈值字段 + `threshold()` helper
  + `QueryGuard` 在 `execute` / `query` + `set_slow_query_threshold` impl

**使用方式**（应用层初始化 subscriber）：

```rust
use tracing_subscriber;

tracing_subscriber::fmt()
    .with_env_filter("rust_ef::query=warn,rust_ef::pool=info,rust_ef::save_changes=info")
    .init();
// 之后所有 rust-ef 操作自动发射 tracing 事件
```

### Added — MySQL TLS 显式 API

新增 `MySqlTlsMode` 枚举，对齐 PostgreSQL 的 `PgTlsMode` 模式，提供显式 TLS 配置：

```rust
pub enum MySqlTlsMode {
    Disabled,        // 禁用 TLS（明文连接）
    Required,        // 强制 TLS（服务器不支持则失败）
    VerifyCa,        // 强制 TLS + CA 证书验证
    VerifyIdentity,  // 强制 TLS + CA 证书 + 主机名验证
}
```

**新增 API**：

- `MySqlProvider::new_with_tls(connection_string, tls: MySqlTlsMode)` — 异步连接 + TLS
- `MySqlProvider::new_lazy_with_tls(connection_string, tls: MySqlTlsMode)` — 延迟连接 + TLS
- `DbContextOptionsBuilderExt::use_mysql_with_tls(connection_string, tls)` — DI 扩展

**设计要点**：

- 独立枚举（不 wrap sqlx 类型），不携带 `TlsConnector`（sqlx 通过 `tls-native-tls`
  feature 内部管理 TLS）
- 排除 `Preferred` 变体（显式 TLS API 不应含"可选"语义）
- CA 证书通过连接串 `ssl-ca` 参数传递
- `From<MySqlTlsMode> for sqlx::mysql::MySqlSslMode` 实现 trait 桥接

**使用示例**：

```rust
use rust_ef_mysql::{DbContextOptionsBuilderExt, MySqlTlsMode};
use rust_ef::DbContextOptionsBuilder;

let mut options = DbContextOptionsBuilder::new();
options.use_mysql_with_tls(
    "mysql://user:pass@host/db?ssl-ca=/path/to/ca.pem",
    MySqlTlsMode::VerifyIdentity,
);
```

### Added — SemVer 严格化迁移指南

新文档 `docs/v1.5-semver-migration-guide.md`：

- v1.0–v1.4 历史 breaking change 清单与迁移路径
- v1.5 起的 SemVer 2.0.0 严格化策略（1 minor deprecation 期）
- `#[deprecated]` 使用规范与编译期警告示例
- v2.0 候选项清单（Arc 元数据共享、async trait 原生语法、错误码体系、metrics API）

### Changed

- workspace 版本 1.4.1 → 1.5.0（6 个 crate 同步）
- `crates/mysql/Cargo.toml`：sqlx features 增加 `tls-native-tls`

### Production readiness

- 可观测性维度：⚠️ → ✅（tracing 集成完成）
- SemVer 维度：⚠️ → ✅（严格化策略落地 + 迁移指南文档化）
- 详见 `docs/PRODUCTION_READINESS_SPEC.md` 第 4.2 节

---

## [1.4.0] — 2026-07-08 — Production hardening (P0+P1) + metadata cache + rust-dix 0.6

### Fixed — P0-1 MySQL `cell_to_string` 类型分发

MySQL Provider 的 `cell_to_string` 之前仅尝试 `String` 反序列化，对非
String 列（BOOL / INTEGER / FLOAT / DATETIME / UUID 等）静默失败并返回
`"NULL"`，导致 `IFromRow::from_row` 解析出错误值。现按 sqlx 类型顺序分发：
bool → i64 → u64 → f64 → NaiveDateTime → NaiveDate → Uuid → String →
Vec\<u8\>，每种类型用 `try_get` 尝试，首个成功者序列化为字符串。覆盖
`crates/mysql/src/connection.rs:20-68`。

### Fixed — P0-2 MetadataCache poison 恢复

`MetadataCache` 使用 `Mutex<HashMap>` 共享进程级元数据缓存。此前
`get_or_build()` 在 mutex 中毒时（前一个持有者 panic）调用
`.expect("MetadataCache poisoned")`，导致整个进程永久不可用。现改为：
捕获 `PoisonError` 后清空缓存中可能不完整的条目，重建后返回。3 个单元
测试覆盖（`test_poison_recovery` / `test_clear_after_poison` /
`test_concurrent_build_safe`），位于 `crates/core/src/metadata_cache.rs`。

### Added — P1-3 SQLite r2d2 连接池

SQLite Provider 引入 `SqliteProviderInner` 枚举区分两种连接策略：

- **`Pooled(r2d2::Pool<SqliteConnectionManager>)`** — 文件模式，默认 8
  连接。每个连接在 acquire 时执行
  `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;`，支持并发读 + 写
  等待，避免 `SQLITE_BUSY` 立即失败。
- **`Single(Arc<Mutex<rusqlite::Connection>>)`** — `:memory:` 模式，保留
  单连接语义以确保测试隔离（SQLite `:memory:` 数据库是 per-connection
  的，必须共享同一连接）。

`SqliteProvider::new(path)` 创建池化 provider；`new_in_memory()` 保留
旧行为。WAL 模式允许读写并发，5s busy_timeout 让写者等待锁释放而非
立即报错。

### Added — P1-6 PostgreSQL TLS 可配置

PostgreSQL Provider 引入 `PgTlsMode` 枚举：

- **`Disable`** — 使用 `tokio_postgres::NoTls`（向后兼容 v1.3，仅用于
  本地开发）
- **`Require(native_tls::TlsConnector)`** — 强制 TLS，使用平台原生实现
  （Windows SChannel / Linux OpenSSL / macOS Secure Transport）

新增 API：

- `PostgresProvider::new_with_tls(connection_string, pool_size, tls: PgTlsMode)`
- `DbContextOptionsBuilderExt::use_postgres_with_tls(connection_string, pool_size, tls)`

TLS 类型在 `deadpool_postgres::Manager` 内部通过 `Box<dyn Connect>`
擦除，因此 `Pool` 与 `PostgresConnection` 保持非泛型 API — TLS 是构造
期决策，非类型参数。依赖 `postgres-native-tls 0.5` + `native-tls 0.2`。

### Added — P1-5 PG/MySQL 集成测试对齐 SQLite 9 场景

`crates/core/tests/common/mod.rs` 此前仅有 `run_crud_lifecycle`
（insert/query/update/delete）。新增 7 个共享 helper 对齐 SQLite 的
9 场景：

- `run_filter_with_in_operator` — linq! 过滤 + IS NULL / IS NOT NULL
- `run_limit_and_offset` — take/skip 分页
- `run_count_and_any` — count + any 存在性检查
- `run_aggregation_queries` — sum/avg via linq! 宏
- `run_empty_result_handling` — 空表 to_list/count/any/first_or_default
- `run_ensure_created_and_deleted` — ensure_created → insert → ensure_deleted → ensure_created 重置
- `run_has_data_seed` — has_data 种子数据在 ensure_created 时物化

PG/MySQL 测试文件各追加 7 个 `#[tokio::test]`，与 SQLite 9 场景对齐
（scenario 5 update/delete 已在 `run_crud_lifecycle` 内）。CI 三库
matrix 全覆盖，本地无 DB 时优雅跳过。

### Added — Process-level metadata cache (priority 1 architecture iteration)

Entity/relationship/settings metadata is now parsed once per `context_key`
and shared as a singleton across all `DbContext` instances created from the
same `DbContextOptions`. Previously, every `from_options()` call (i.e. every
HTTP request via `get_owned()`) re-iterated `inventory::iter` and re-ran
all `IEntityTypeConfiguration::configure()` callbacks.

- **New module** `crates/core/src/metadata_cache.rs`: `MetadataCache` (a
  `Mutex<HashMap<Option<String>, Arc<BuiltMetadata>>>` keyed by `context_key`)
  + `BuiltMetadata` (snapshots `entity_metas`, `model_metas`, `configs`).
- **`DbContextOptions`** gains a `metadata_cache: Arc<MetadataCache>` field.
  Since `DbContextOptions` is already `Arc`-shared per `add_dbcontext`
  registration, the cache is naturally singleton-per-registration.
- **`DbContext::from_options()`** now calls `metadata_cache.get_or_build()`
  instead of re-iterating inventory. First call builds; subsequent calls
  `Arc::clone` the cached `BuiltMetadata`.
- **`ModelBuilder::from_built()`** new `pub(crate)` constructor populates
  `entity_metas` + `configs` from the cache, leaving `build_cache` /
  `filter_cache` lazy. Per-instance mutations (`has_query_filter`, etc.)
  only affect that `ModelBuilder` instance — the cache is never mutated.
- **`DbContext::discover_entities()`** is now a no-op (method retained for
  backward compatibility; metadata is pre-populated by `from_options()`).
- **Removed**: `DbContext.context_key` field (orphaned by the no-op
  `discover_entities()`; the key is read from `DbContextOptions` instead).
- **`EntityConfig` / `PropertyConfigOverride`** changed from private to
  `pub(crate)` so `MetadataCache::build()` can snapshot them.

#### Performance impact

For a 10-entity application, `from_options()` after the first call skips
~10 `configure()` callback executions + ~10 `EntityTypeMeta` constructions
+ 2 `inventory::iter` traversals. Expected ~90% reduction in per-request
metadata setup cost.

#### What is NOT shared (v1 limitation)

- `EntityTypeMeta.property_index` / `navigation_index` `OnceLock` caches
  are per-DbContext (each context clones its own `EntityTypeMeta` from the
  cache). The rebuild cost (~10-20 HashMap insertions per entity) is
  negligible against DB I/O. Arc-wrapping `EntityTypeMeta` is deferred to v2.

#### Backward compatibility

- `from_options()`, `set::<T>()`, `save_changes()`, `model()` public APIs
  are unchanged.
- `discover_entities()` still compiles and runs (as a no-op).
- `multi_db_context_tests` (context_key isolation) still passes — the cache
  is keyed by `context_key`.

### Added — Transaction interface extension (priority 2)

Extends `IAsyncConnection` with savepoint and isolation level capabilities,
and introduces an **ambient transaction** mechanism on `DbContext` so that
`save_changes()` can reuse a transaction opened by the caller — aligning
with EFCore's `Database.BeginTransaction()` / `Transaction.Commit()` pattern.

- **New `IsolationLevel` enum** (`ReadUncommitted` / `ReadCommitted` /
  `RepeatableRead` / `Serializable`) in `crates/core/src/provider.rs`,
  re-exported via `prelude`.
- **`IAsyncConnection` trait**: 4 new required methods —
  `create_savepoint(name)`, `release_savepoint(name)`,
  `rollback_to_savepoint(name)`, `set_transaction_isolation(level)`.
  No default implementations because SQL dialects differ (SQLite uses
  `RELEASE name` / `PRAGMA read_uncommitted`; PG/MySQL use
  `RELEASE SAVEPOINT name` / `SET TRANSACTION ISOLATION LEVEL`).
- **Provider implementations**: SQLite, PostgreSQL, MySQL each implement
  the 4 new methods with dialect-correct SQL.
- **`DbContext` ambient transaction**:
  - New field `ambient_transaction: Option<Box<dyn IAsyncConnection>>`.
  - `begin_transaction(&mut self) -> EFResult<()>` opens a transaction and
    stores it; subsequent `save_changes()` calls reuse this connection and
    do not begin/commit/rollback on their own (uses `take()`/restore pattern
    to avoid `&mut self` borrow conflicts with `self.sets`).
  - New `commit_transaction(&mut self)` / `rollback_transaction(&mut self)`.
  - New `create_savepoint` / `release_savepoint` / `rollback_to_savepoint` /
    `set_transaction_isolation` proxy methods (require active ambient
    transaction; return `EFError::Transaction` otherwise).
- **`save_changes()` integration**: when `ambient_transaction` is `Some`,
  takes the connection, runs all DbSet saves, and restores it without
  committing (the outer scope controls commit/rollback). When `None`,
  behaves as before (self-managed transaction).

### Breaking — `DbContext::begin_transaction` signature change

```rust
// Before:
pub async fn begin_transaction(&self) -> EFResult<Box<dyn IAsyncConnection>>

// After:
pub async fn begin_transaction(&mut self) -> EFResult<()>
```

The old signature returned a raw connection with no ambient tracking —
`save_changes()` could not reuse it (especially in PG/MySQL where each
`get_connection()` returns a different pooled connection). The new signature
stores the transaction in `DbContext` so `save_changes()` can reuse it.
Verified no external callers via grep.

### Migration — priority 2

1. Replace `let conn = ctx.begin_transaction().await?;` with
   `ctx.begin_transaction().await?;` (no return value).
2. Use `ctx.commit_transaction().await?` / `ctx.rollback_transaction().await?`
   to close the ambient transaction.
3. `save_changes()` called between `begin_transaction` and `commit_transaction`
   now reuses the ambient transaction (no code change needed).
4. New savepoint/isolation APIs are available on `DbContext` directly:
   `ctx.create_savepoint("sp1").await?` etc.

### Changed — linq.rs subdirectory split

Split `crates/macros/src/linq.rs` (2643 lines) into a `linq/` subdirectory
with 6 child modules for clearer responsibility separation:

- `ast.rs` (175 lines) — AST types (`LinqInput`, `QueryInput`, `LinqClause`,
  `HavingExprAst`)
- `parse.rs` (965 lines) — `impl Parse` + all `parse_*` functions + `ValueKind`
  + `JoinKind`
- `context.rs` (186 lines) — `LinqCtx` + `FieldKind` + `FieldRef` + field
  extraction helpers
- `compile.rs` (784 lines) — `compile_bool_expr` / `compile_expr` /
  `compile_method` / `compile_order` / `compile_having_expr` + subquery
  compilation
- `expand.rs` (412 lines) — `expand_linq` entry point + `expand_clauses` +
  `expand_join` (code generation)
- `mod.rs` (11 lines) — module declarations + `pub use expand::expand_linq`

Fixed E0027 (non-exhaustive match) in `expand_clauses`: the `LinqClause::With`
arm now destructures all 6 fields (`name`, `entity`, `param`, `body`,
`recursive`, `link`) and generates recursive CTE SQL via
`with_recursive_cte_typed` when `recursive` is true.

All internal items use `pub(crate)` visibility; only `expand_linq` is
re-exported via `pub use`. `crates/macros/src/lib.rs` unchanged — `mod linq;`
transparently resolves to `linq/mod.rs`.

---

## [1.3.1] — 2026-07-07 — rust-dicore → rust-dix 0.6 rename + breaking API sync

`rust-dicore` has been renamed to `rust-dix` upstream. The 0.6.0 release on
crates.io is the renamed successor of `rust-dicore 0.5.1`, but it also
introduces **breaking API changes** beyond the rename. rust-ef has been
migrated to rust-dix 0.6 and updated to match the new resolution API.

### Changed — rust-dix 0.6 sync

- **`rust-dicore` renamed to `rust-dix`** (upstream): crate name, dependency
  declaration, and the `rust_dix::` import path (formerly `rust_dicore::`).
- **Dependency bump**: `rust-dicore = "0.5.1"` → `rust-dix = "0.6"` in
  `crates/core/Cargo.toml`.
- **Import path**: `rust_dicore::*` → `rust_dix::*` in `crates/core/src/di.rs`
  and test files.
- **Re-exports**: `pub use rust_dix::{ServiceCollection, ServiceProvider}` in
  `di.rs`.

### Breaking — rust-dix 0.6 resolution API (affects user code)

The following changes in rust-dix 0.6 affect all rust-ef consumers that call
`provider.get()`, `provider.get_owned()`, or `provider.get_keyed_owned()`
directly. `#[derive(Inject)]`-generated constructors are unaffected — the
macro emits the correct unwrap internally.

1. **`ServiceCollection::build()` now returns `Arc<ServiceProvider>` directly**
   (previously returned `ServiceProvider`, requiring user code to wrap in
   `Arc::new()`). Remove the manual `Arc::new()` wrap:
   ```rust
   // Before (rust-dicore 0.5.1):
   let provider = Arc::new(ServiceCollection::new().add_dbcontext(...).build().unwrap());

   // After (rust-dix 0.6):
   let provider: Arc<ServiceProvider> = ServiceCollection::new().add_dbcontext(...).build().unwrap();
   ```

2. **`get()` / `get_owned()` / `get_keyed_owned()` now return `Result<_, RdiError>`**
   (previously returned the value directly, panicking on failure). Add
   `.unwrap()` or `.expect("...")` (or `?` in functions returning `Result`):
   ```rust
   // Before:
   let ctx: DbContext = provider.get_owned();
   let ctx: Arc<DbContext> = scope.get();

   // After:
   let ctx: DbContext = provider.get_owned()?;
   let ctx: Arc<DbContext> = scope.get()?;
   ```

3. **`create_scope()` moved to the `ScopeFactory` trait** — must be imported
   before calling:
   ```rust
   use rust_dix::scope::ScopeFactory;
   let scope = provider.create_scope();
   ```
   Alternatively, use the inherent `.scope()` method on `ServiceProvider`,
   which does not require a trait import.

### Migration — 1.3.x → Unreleased (rust-dix 0.6)

1. **Cargo.toml**: replace `rust-dicore = "0.5.1"` with `rust-dix = "0.6"`.
2. **Imports**: replace `use rust_dicore::*;` with `use rust_dix::*;` and
   `rust_dicore::ServiceCollection` with `rust_dix::ServiceCollection`.
3. **Provider construction**: drop the `Arc::new()` wrap around
   `ServiceCollection::...build().unwrap()`. `build()` already returns
   `Arc<ServiceProvider>`.
4. **Resolution calls**: append `?` (or `.unwrap()` / `.expect("...")`) to all
   `provider.get()`, `provider.get_owned()`, `provider.get_keyed_owned()`,
   `scope.get()`, `scope.get_owned()`, `scope.get_keyed_owned()` calls.
5. **Scope creation**: add `use rust_dix::scope::ScopeFactory;` before calling
   `provider.create_scope()` (or switch to `.scope()`).
6. **Attribute macros** (if used directly): `#[rust_dicore::inject]` →
   `#[rust_dix::inject]`. rust-ef itself does not use this attribute directly;
   handler structs use `#[derive(Inject)]` from `rust-ef-macros` which is
   unaffected.
7. **Behavior**: identical — ServiceProvider is still the root scope, Scoped
   caches per-scope, `get_owned()` bypasses the cache, `from_injected()`
   collects `#[inject]`-annotated services via `inventory`.

### Documentation

- Updated all `rust-dicore` / `rust_dicore` references in `di.rs`,
  `crates/core/README.md`, top-level `README.md`, and `docs/rust-ef/**` to
  `rust-dix` / `rust_dix`.
- Updated doc examples to reflect the new `Result`-returning resolution API
  (`get_owned().unwrap()` / `get_owned()?` instead of bare `get_owned()`).
- Noted the `ScopeFactory` trait import requirement in scope-creation
  examples.

---

## [1.3.1] — 2026-06-30 — rust-dicore 0.5.1 sync

Upgrades to `rust-dicore 0.5.1`. The 0.5.1 macros enforce **explicit field
marking** (LRDI rule 8): bare `T` fields MUST be marked `#[inject(owned)]` and
`Arc<T>` fields MUST be marked `#[inject]`. Unmarked fields fall back to
`Default::default()`. The previous 0.5.0 "auto-detect bare T" behavior is
removed — this is a breaking change for any handler struct that relied on
implicit owned resolution.

### Changed — 0.5.1 sync

- **`rust-dicore` upgraded from 0.5.0 to 0.5.1**: `rust-dicore-macros` also
  upgraded to 0.5.1. The `gen_field_init` macro now treats unmarked fields as
  internal state (`Default::default()`), matching the documented LRDI rule 8.
- **Handler structs require explicit `#[inject(owned)]`**: every bare
  `ctx: DbContext` field in `#[derive(Inject)]` structs MUST now be marked
  `#[inject(owned)]`. `Arc<T>` fields MUST be marked `#[inject]`.
- **Documentation**: updated all handler examples across `di.rs`,
  `db_context.rs`, README files, `docs/rust-ef/**`, and the `lref` skill
  templates/references to reflect the explicit marking requirement. Fixed
  `rust_dicore::` fully-qualified paths to `use rust_dicore::*;` (LRDI rule 6).
- **`lref` skill**: fixed `keyed_transient` → `keyed_scoped` in
  `architecture.md`; fixed `rust_dicore::ServiceProvider` fully-qualified path
  in `di-setup.rs` template.

### Migration — 1.3.0 → 1.3.1 (0.5.1 sync)

1. **Handler structs**: add `#[inject(owned)]` to every bare `T` field
   (e.g. `ctx: DbContext`). Add `#[inject]` to every `Arc<T>` field.
2. **Unmarked fields**: confirm they implement `Default` — unmarked fields now
   use `Default::default()` instead of auto-detection.
3. **Imports**: ensure `use rust_dicore::*;` at the file top — fully-qualified
   `rust_dicore::` paths are forbidden (LRDI rule 6).

### Added — 0.5.1 (upstream)

- `try_get_owned::<T>() -> Option<T>` — non-panicking owned resolution
  (returns `None` for unregistered or Singleton services).
- `Option<T>` field support in `#[derive(Inject)]` → resolves via
  `try_get_owned`; `Option<Arc<T>>` field → resolves via `try_get`.

---

## [1.3.0] — 2026-06-29 — Owned Resolution + rust-dicore 0.5.0

Upgrades to `rust-dicore 0.5.0` with owned resolution support, eliminating the
`Arc<DbContext>` + `&mut self` tension without interior mutability. Handlers
now own `DbContext` directly via `get_owned()`, enabling idiomatic `&mut self`
access — no `Arc<Mutex>`, no locks, no `unsafe`.

### Added — v1.3

- **Owned resolution**: `IServiceResolver::get_owned::<T>() -> T` bypasses
  the DI cache and returns a fresh owned instance. `#[derive(Inject)]`
  auto-detects bare `T` fields (vs `Arc<T>`) and resolves them via
  `get_owned()`. Scoped/Transient services support owned resolution;
  Singleton returns `None` (shared instance cannot be owned).
- **End-to-end handler pattern**: `#[inject(scoped)]` + `ctx: DbContext`
  (bare field) + `handle(&mut self)` + `self.ctx.set::<T>()` /
  `self.ctx.save_changes()`. Each request gets a fresh `DbContext` —
  aligned with EFCore + ASP.NET Core DI semantics.
- **Keyed owned resolution**: `get_keyed_owned::<T>("key")` for multi-DB
  scenarios.
- 4 integration tests in `owned_injection_tests.rs` verifying the complete
  flow: `#[inject(scoped)]` → `get_owned::<Handler>()` → `handle(&mut self)`
  → `self.ctx.set::<T>().add()` → `self.ctx.save_changes()`.
- 2 new scoped lifecycle tests: `owned_resolution_returns_fresh_instance`
  and `owned_resolution_bypasses_scope_cache`.

### Changed — v1.3

- **`rust-dicore` upgraded from 0.3.2 to 0.5.0**: ServiceProvider is now the
  root scope — Scoped services resolved from root are cached in
  `root_scoped_cache` (same instance per call), matching EFCore semantics.
  Previously root resolution degraded to transient.
- **Handler templates**: All examples updated from `ctx: Arc<DbContext>` +
  `handle(&self)` to `ctx: DbContext` (owned) + `handle(&mut self)` +
  `self.ctx.` prefix. Fixed template bug where `ctx.` was used without
  `self.` prefix in handler methods.
- **`#[inject(scoped)]` requirement**: Handler trait impls MUST use
  `#[inject(scoped)]`, not bare `#[inject]` (which defaults to Singleton
  and causes captive dependency errors with the Scoped `DbContext`).
- **Documentation**: Comprehensive updates across `di.rs`, `db_context.rs`,
  `webapp-integration.md`, `quickstart.md`, `architecture.md`,
  `advanced.md`, `pitfalls.md`, `di-registration.md`, `keyed-databases.md`,
  `dbcontext-and-di.md`, `multi-tenancy-foundation.md`, README files, and
  skill templates. Fixed UTF-8 encoding corruption in multiple doc files.

### Migration — v1.2 → v1.3

1. **Handler structs**: Change `ctx: Arc<DbContext>` to `ctx: DbContext`
   (bare field). `#[derive(Inject)]` auto-detects and uses `get_owned()`.
2. **Handler methods**: Change `handle(&self, ...)` to `handle(&mut self, ...)`
   and prefix all context calls with `self.` (e.g. `self.ctx.set::<T>()`).
3. **Handler registration**: Change `#[inject]` to `#[inject(scoped)]` on
   trait impl blocks to avoid captive dependency errors.
4. **Root resolution**: `provider.get::<DbContext>()` now returns the same
  instance per call (root scope cache). Use `provider.get_owned::<DbContext>()`
  for a fresh instance each call.
5. **`Arc<DbContext>` still supported** for shared `&self`-only scenarios
   (e.g. `ensure_created()`), but cannot call `set::<T>()` or
   `save_changes()` (requires `&mut self`).

---

## [1.1.0] — 2026-06-27 — Query Fidelity + Entity Auto-Discovery

Enhances query expressiveness with lazy loading, native type binding fixes,
dialect bug fixes, IN/NOT IN subqueries, CTE / window function support,
CTE syntax sugar for the `linq!` macro, and introduces automatic entity
discovery with multi-database context key support.

### Added — v1.1

- **Lazy Loading** (opt-in): `DbContextOptionsBuilder::use_lazy_loading(bool)`
  flag propagated through `DbSet` → `QueryBuilder` → `to_list()`. Navigation
  containers (`BelongsTo` / `HasMany` / `HasOne`) gain `is_loaded()`,
  `set_lazy_context()`, and async `load()` methods. `ILazyInit` trait
  auto-generated by `#[derive(EntityType)]`; `MAX_LAZY_DEPTH = 16` recursion
  guard. 7 integration tests in `lazy_loading_tests.rs`.
- **IN / NOT IN subquery support**: `b.field.in_subquery(|p: Post| p.blog_id)`
  syntax in `linq!` macro (Forms A/B and C). New `BoolExpr::InSubquery` /
  `BoolExpr::NotInSubquery` variants and `InSubquerySpec` struct. 6 integration
  tests in `in_subquery_tests.rs`.
- **CTE (Common Table Expressions)**: Two usage modes:
  - **Runtime API (raw mode)**: `QueryBuilder::with_cte_internal(name, sql,
    params, columns)` and `QueryBuilder::from_cte(name)`. CTE parameters are
    prepended to the query parameter vector; `WITH name AS (...)` prefix
    emitted before SELECT. Explicit column lists supported
    (`WITH name (c1, c2) AS (...)`).
  - **`linq!` macro syntax sugar (typed mode)**: `linq!(with name as |e: T|
    ...; from name)` compiles the closure body into a `BoolExpr` via
    `compile_bool_expr`, generating a type-safe CTE whose body
    `SELECT * FROM <table> WHERE <expr>` uses provider-correct placeholders
    (`?` on SQLite/MySQL, `$N` on PostgreSQL). Eliminates raw SQL strings,
    manual parameter management, and PostgreSQL placeholder mismatches.
    `QueryBuilder::with_cte_typed(name, table, where_expr)` is the underlying
    builder method. 9 integration tests in `cte_syntax_tests.rs`.
- **Window functions**: `linq!(window ...)` clause with 10 function kinds
  (`ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`, `SUM`, `COUNT`, `AVG`,
  `MIN`, `MAX`). `WindowSpec` AST with `PARTITION BY` and `ORDER BY` support.
  `WindowFuncKind` enum and `WindowSpec::to_sql()` compile window projections
  with dialect-specific identifier quoting. 12 integration tests in
  `window_function_tests.rs`.
- `QueryState::all_params()` method returning CTE params ++ WHERE/HAVING params
  in correct placeholder order.
- `WindowFuncKind`, `WindowSpec`, `CteSpec` exported in `prelude`.
- **Automatic entity discovery**: `DbContext::from_options()` now
  auto-discovers all entities registered via `#[derive(EntityType)]` and
  applies all `#[entity(T)]` Fluent configurations. Developers no longer
  need to manually call `ctx.discover_entities()`. The call is idempotent —
  manual `discover_entities()` calls after `from_options()` are safe no-ops.
- **Multi-database context key support**:
  - `#[context("key")]` attribute on entity structs tags them for a
    specific keyed `DbContext`. Entities without this attribute default to
    the default context (`None`).
  - `#[entity(T, "key")]` attribute on config impls applies Fluent
    configurations only to the matching keyed context.
  - `DbContextOptionsBuilder::context_key(key)` sets the context key on
    options. `add_dbcontext_keyed(key, ...)` sets it automatically.
  - `discover_entities()` filters entity registrations and config
    registrations by the context's key, ensuring each `DbContext` only
    manages entities belonging to its context.
- **`#[entity(T)]` macro** (renamed from `#[entity_config(T)]`): The
  attribute macro for `impl IEntityTypeConfiguration<T>` blocks is now
  named `#[entity(T)]` for a cleaner API. The old name `#[entity_config(T)]`
  has been removed without a deprecated alias.
- `DbContext::model_builder()` read-only accessor and
  `DbContext::entity_metas_contains::<T>()` type-check method.
- 6 integration tests in `multi_db_context_tests.rs` covering context key
  tagging, filtering, and Fluent config override isolation.

### Fixed — v1.1

- **PostgreSQL HAVING placeholder bug**: `HavingExpr::to_sql` hardcoded `?`
  placeholder. Fixed to use `ISqlGenerator::parameter_placeholder()` with
  shared `param_idx` for contiguous `$N` numbering across WHERE and HAVING.
- **PostgreSQL LIMIT/OFFSET dialect bug**: `to_sql_with` didn't use
  `gen.pagination()`. Fixed to delegate to the dialect-specific generator so
  PostgreSQL emits `OFFSET x LIMIT y` and MySQL handles offset-only via
  sentinel LIMIT.
- **PostgreSQL native chrono/uuid parameter binding**: `DbValue` now preserves
  `chrono::DateTime` / `chrono::NaiveDateTime` / `uuid::Uuid` types instead of
  collapsing to `String`, enabling native PostgreSQL type inference.
- **PostgreSQL multi typed CTE placeholder collision**: `to_sql_with` reset
  `cte_idx` to 1 inside the CTE `.map()` closure, causing multiple typed CTEs
  to emit duplicate `$1` placeholders on PostgreSQL. Fixed to use a
  `running_idx` that accumulates across CTEs so `$N` stays contiguous with
  `all_params()` order. Regression covered by 4 new PG dialect tests in
  `cte_syntax_tests.rs`.

### Changed — v1.1

- `QueryBuilder::to_list` and all terminal methods now require `T: ILazyInit`
  bound (auto-satisfied by `#[derive(EntityType)]`).
- `QueryState` gains `windows: Vec<WindowSpec>` and `ctes: Vec<CteSpec>` fields.
- `CteSpec` gains `table: String` and `where_expr: Option<BoolExpr>` fields to
  support typed mode, and is now `#[non_exhaustive]` to prevent direct
  construction outside the crate. Existing `with_cte_internal` API is
  unchanged (new fields default to empty). Use `with_cte_internal` /
  `with_cte_typed` to create CTE specifications.
- `to_sql_with` emits window function projections in the SELECT list and CTE
  `WITH` prefix before SELECT; `param_idx` starts at `1 + cte_param_count` for
  PostgreSQL placeholder continuity. Typed CTE bodies are compiled at this
  point via `compile_bool_expr` with provider-correct placeholders.
- All execution sites updated to use `QueryState::all_params()` for CTE
  parameter ordering.
- `DbContext::from_options()` now automatically calls `discover_entities()`,
  populating entity metadata and applying `#[entity(T)]` Fluent configurations
  without requiring manual registration. The call is idempotent — subsequent
  `discover_entities()` calls are safe no-ops.
- `discover_entities()` filters `EntityRegistration` and
  `EntityConfigRegistration` entries by the context's `context_key`, ensuring
  each `DbContext` only manages entities belonging to its context.
- `EntityRegistration` and `EntityConfigRegistration` gain a
  `context_key: Option<&'static str>` field for multi-database context
  filtering. Existing inventory submissions generated by older macro versions
  remain source-compatible because the field is populated by the macro, not by
  user code.
- `DbContextOptions` and `DbContextOptionsBuilder` gain a `context_key`
  field/setter; `DbContext` stores a `context_key: Option<String>` for use
  during `discover_entities()` filtering.

### Removed — v1.1

- **`#[entity_config(T)]` attribute macro** (breaking change): renamed to
  `#[entity(T)]` for a cleaner API. The old name has been removed **without a
  deprecated alias** to keep the framework's final shape clean. Migrate by
  renaming `#[entity_config(T)]` to `#[entity(T)]` (and
  `#[entity_config(T, "key")]` to `#[entity(T, "key")]` for keyed contexts).
  The `rust_ef::entity_config` re-export is gone; use `rust_ef::entity` (or the
  prelude) instead.

---

## [1.0.0] — 2026-06-27 — General Availability

The first production-ready release. The framework is feature-complete for
EF Core-style ORM workflows on SQLite, PostgreSQL, and MySQL, with stable
public APIs and a comprehensive documentation set.

### Highlights

- **Stable API surface**: no `#[deprecated]` residue; `EFError` / `EFResult`
  unified naming. Workspace version bumped to `1.0.0` across all crates.
- **mdBook documentation site** with full-text search, dark theme, and
  automatic GitHub Pages deployment on every push to `main`.
- **Security audit passed**: all runtime values parameterized through
  `DbValue`; identifiers sourced exclusively from compile-time entity
  metadata. See `docs/rust-ef/11-best-practices/security.md`.
- **Criterion performance benchmarks** for batch INSERT / SELECT and
  Include vs N+1 comparison.

### Added — 1.0 GA

- `docs/rust-ef/book.toml` mdBook configuration with search, fold, and
  dark theme (`navy`) defaults.
- `docs/rust-ef/SUMMARY.md` complete table of contents spanning 11
  chapters plus foreword and appendix.
- `.github/workflows/docs.yml` GitHub Pages deployment workflow using
  `peaceiris/action-mdbook` and `actions/deploy-pages@v4`.
- `docs/rust-ef/11-best-practices/security.md` six-section security
  guide: SQL injection defense, migration trust model, connection-string
  handling, sensitive-field mapping, multi-tenant filters, and a
  production hardening checklist.
- `crates/core/benches/bench_insert.rs`, `bench_query.rs`,
  `bench_include.rs` — Criterion `async_tokio` benchmarks parameterized
  over 100/500/1000 rows and 50×10 Include load.
- `CHANGELOG.md` (this file).

### Changed — 1.0 GA

- Workspace `version = "0.3.5"` → `"1.0.0"` in `[workspace.package]`;
  propagated to every inter-crate dependency
  (`rust-ef-macros`, `rust-ef-sqlite`, `rust-ef-postgres`, `rust-ef-mysql`).
- `README.md` Quick Start dependencies updated from `rust-ef = "0.3"` to
  `rust-ef = "1.0"`; added documentation badge and online docs link.
- `docs/PRODUCTION_READINESS_SPEC.md` readiness 98% → 100%, all 1.0 GA
  acceptance criteria marked complete.
- `.gitignore` now excludes `docs/rust-ef/book/` mdBook build output.

### Removed — 1.0 GA

- Deprecated type aliases `LrefError` and `LrefResult` from
  `crates/core/src/error.rs`. Use `EFError` / `EFResult` instead.

### 1.0 GA Acceptance Criteria

| Criterion | Status |
|-----------|:------:|
| chrono + uuid type support | ✅ |
| mdBook docs accessible online | ✅ |
| Performance benchmark report | ✅ |
| Security audit passed | ✅ |
| API stable, no deprecated residue | ✅ |
| ≥ 3 example projects | ✅ (`blog`, `soft_delete`, `audit`) |
| 1.0.0 release | ✅ |

---

## [0.5] — 2026-06-26 — Release Candidate 1

Navigation / advanced features fully ready plus the CLI migration tool.
Overall readiness reached ~98%, removing all P0 blockers for 1.0 GA.

### Added

- **Optimistic concurrency**: `ChangeExecutor::execute_updates` and
  `execute_deletes` now append the `#[concurrency_check]` token column to
  the WHERE clause using the original snapshot value; `rows_affected == 0`
  returns `EFError::ConcurrencyConflict`. Six end-to-end tests in
  `concurrency_tests.rs`.
- **CLI crate** (`rust-ef-cli`) with subcommands:
  - `migration add <Name> --output ./Migrations` — emit migration file
    skeleton.
  - `migration list --connection ... --provider sqlite|postgres|mysql` —
    print applied vs pending migrations.
  - `migration apply --connection ... --provider ...` — apply all
    pending migrations and record history.
  - `migration revert --connection ... --target <Name>` — roll back to
    the specified migration.
  - `migration script --from X --to Y` (or `--name SingleMigration`) —
    emit forward/reverse SQL script.
  - `scaffold dbcontext` — generate entity source from an existing
    database schema.
- **Library migration API**:
  - `MigrationEngine::apply_pending()` reads `__ef_migrations_history`
    and applies only pending migrations.
  - `revert()`, `revert_last()`, `revert_to_target()`.
  - `generate_script(from, to)` produces forward and reverse SQL.
  - `get_applied_migrations()` introspection helper.
- **FK / index diff** in `SchemaDiffer`:
  - `SchemaChange::AddForeignKey` / `DropForeignKey` integrated into
    `diff()`; generates `ALTER TABLE ... ADD CONSTRAINT` / `DROP
    CONSTRAINT`.
  - `SchemaChange::CreateIndex` / `DropIndex` integrated; SQLite/PG use
    `IF EXISTS`, MySQL uses `ON table` syntax.
  - `SnapshotColumn` carries `has_index` / `is_unique`; index diff fields
    excluded from `columns_structurally_equal` to avoid spurious
    `AlterColumn` operations. Ten tests in `index_diff_tests.rs`.
- **Subqueries / correlated filtering** via `any` / `none` / `all`
  helpers compiled to `EXISTS` / `NOT EXISTS`. Eight tests in
  `subquery_tests.rs`.
- **Global query filters**:
  - `ModelBuilder::has_query_filter` accepts `BoolExpr` from `linq!`.
  - `query_ignore_filters()` for administrator queries.
  - UPDATE/DELETE WHERE clauses also constrained by the filter.
  - Four tests in `query_filter_exec_tests.rs`.
- **Chrono / uuid / decimal optional features**:
  - `chrono` feature: `DateTime<Utc>`, `NaiveDateTime`, `NaiveDate`
    mapped to RFC3339 / `"YYYY-MM-DD HH:MM:SS"` / `"YYYY-MM-DD"`.
  - `uuid` feature: `uuid::Uuid` (with `v4`).
  - `decimal` feature: `rust_decimal::Decimal`.
  - Three dialect DDL mappings (PG `TIMESTAMPTZ`/`UUID`/`NUMERIC`;
    MySQL `DATETIME`/`CHAR(36)`/`DECIMAL(38,18)`; SQLite `TEXT`).
  - Six feature-gated tests in `extended_types_tests.rs`.
- **`exists_by_id` / `exists_by_key`** convenience methods on
  `IQueryable<T>` returning `EFResult<bool>` via `SELECT 1 ... LIMIT 1`.
  Eight tests in `exists_by_id_tests.rs`.
- **Transaction rollback + composite primary key CRUD** integration
  tests in `transaction_composite_tests.rs` (six tests).
- **GitHub Actions CI** with three-database matrix (SQLite in-process;
  PostgreSQL 16 and MySQL 8 in service containers). Lint job runs
  `cargo fmt --check` and `cargo clippy -- -D warnings` for default and
  `chrono,uuid,decimal` feature sets.
- **Soft delete and audit interceptor examples** under `examples/`.

### Changed

- `DbContext` DI registration now supports `add_dbcontext`,
  `add_dbcontext_keyed`, and `add_dbcontext_from_options`.
- README updated with modern Quick Start, multi-DB keyed registration,
  and SaveChanges interceptor snippet.
- `crates/core/src/error.rs` consolidated around the `EFError` / `EFResult`
  naming; legacy `LrefError` / `LrefResult` aliases marked `#[deprecated]`
  (removed in 1.0).

### Documentation

- All `docs/rust-ef/` chapters refreshed to reflect v0.5 behavior;
  `⚠️` markers removed or annotated with concrete follow-up tasks.

---

## [0.4] — 2026-06-22 — Beta 1

Full CRUD chain and query completeness; example projects modernized.

### Added

- **Modern `examples/blog`** rewritten around the type-map DbContext:
  `ctx.set::<Blog>()` + `ctx.save_changes()`, `linq!` queries, Include
  navigation, bulk operations, and `add_dbcontext` DI registration.
- **SQLite integration test suite expansion**: transaction rollback,
  multi-entity save, composite primary key CRUD, full type mapping
  (bool / Option / String / i32 / f64), global-filter + `linq!`
  combinations.
- **PostgreSQL / MySQL integration tests** under
  `crates/core/tests/postgres_crud_tests.rs` and
  `mysql_crud_tests.rs`, sharing a `tests/common/mod.rs` CRUD lifecycle
  helper. CI matrix executes all three databases in parallel.
- **Crate README consolidation** — every crate's README rebranded to
  `rust-ef-*` (replacing legacy `lref` references).
- **`cargo clippy -- -D warnings`** added to CI; zero warnings across
  core + three providers.

### Changed

- `find_by_id` renamed to `find` based on primary-key metadata.
- `set_property` dead code removed.
- 14 string-based query APIs renamed to `*_internal` with
  `#[doc(hidden)] pub` and `&'static str` constants, replacing the
  removed `*_named` / `filter_raw` surfaces.

---

## [0.3.5] — 2026-06-15 — DSL Unification

`linq!` macro becomes the single entry point for all query and DML
operations; all string-based APIs removed without deprecation transition.

### Added

- **`linq!` macro three forms**:
  - **Form A** — filter closure (reusable expression tree or direct
    query): `linq!(|b: Blog| b.rating > 0.5)`.
  - **Form B** — multi-clause query (`;`-separated): `include`,
    `order_by`, `group_by`, `having`, `select`, `inner_join` /
    `left_join`, `sum` / `avg` / `min` / `max` / `count`, `set` +
    `execute_update`, `take` / `skip`, etc.
  - **Form C** — value producer for `ModelBuilder` configuration:
    `filter`, `index`, `key`.
- **`LinqClause` enum** covering all query semantics; `expand_query`
  unifies expansion.
- **LINQ terminal methods**: `last`, `last_or_default`, `single`,
  `single_or_default`, `long_count`, `all`, `contains`,
  `to_dictionary`.
- **`ModelBuilder` DSL**: `has_query_filter` accepts `BoolExpr`;
  `has_index` / `has_key` accept `&'static [&'static str]` produced by
  `linq!(index ...)` / `linq!(key ...)`.

### Removed

- String-based APIs `include_named`, `then_include_named`,
  `set_column`, `filter_raw`, `sum("col")`, `avg("col")`,
  `inner_join(...)`, `group_by(&[...])`, `having("...")` removed
  immediately (no `#[deprecated]` transition). All user code must
  migrate to the `linq!` macro.

---

## [0.3] — 2026-05-20 — Type-Map DbContext

Architectural refactor: `DbContext` no longer holds typed `DbSet<T>`
fields. Sets are lazily created via `set::<T>()` against a type-map,
enabling generic `save_changes()` iteration.

### Added

- **Type-map `DbContext`** with `ctx.set::<T>()` lazy initialization.
- **`Arc<dyn IDbContext>` DI** integration with `rust-dicore`.
- **`SetOps<T>` type-erased dispatcher** — `save_changes()` iterates
  all registered entity types without per-entity code generation.
- **`IDbContext` object-safe trait** with `provider()`,
  `save_changes()`, `change_tracker()`.
- **`IDbContextExt`** for non-object-safe generic helpers such as
  `use_transaction(f)`.
- **Keyed multi-database registration** via `add_dbcontext_keyed` and
  `provider.get_keyed("name")`.
- **`FromDbContextOptions` DI bridge** with `from_options(&DbContextOptions) -> Self`.
- **SaveChanges interceptor pipeline**: `ISaveChangesInterceptor`
  with `on_saving`, `on_saved`, `on_save_failed` hooks.
- **`MigrationEngine`** library API with model-snapshot diff,
  three-dialect Up/Down SQL generation, and history tracking.
- **Three providers**: `rust-ef-sqlite`, `rust-ef-postgres`,
  `rust-ef-mysql`, each following the unified module structure
  (`sql_generator.rs`, `provider.rs`, `connection.rs`,
  `type_conversion.rs`, `type_mapping.rs`, `introspection.rs`,
  `di_extension.rs`).

---

## [0.2] — 2026-04-10 — Alpha 2

Early scaffold with manual per-entity `save_changes` and a `linq!`
prototype.

### Added

- `#[derive(EntityType)]` macro with 12 attributes (`table`,
  `primary_key`, `auto_increment`, `required`, `max_length`, `column`,
  `foreign_key`, `navigation`, `not_mapped`, `index`, `unique`,
  `concurrency_check`).
- Initial `BoolExpr` AST (Filter / Raw / And / Or / Not) and IN /
  BETWEEN / IS NULL / `contains` support.
- SQLite provider with CRUD lifecycle test coverage.

---

## [0.1] — 2026-03-01 — Initial Alpha

Project skeleton, workspace layout, and the `IDbContext` / `IDbSet<T>`
/ `IEntityType` trait hierarchy.

### Added

- Workspace with `crates/core`, `crates/macros`, `crates/sqlite`,
  `crates/postgres`, `crates/mysql`, `crates/cli`.
- `IEntityType` / `IFromRow` / `IGetKeyValues` / `IEntitySnapshot`
  trait hierarchy.
- `IDatabaseProvider` abstraction with `ISqlGenerator`,
  `IAsyncConnection`, `execute_migration_command(sql)`.
- Initial `MigrationStore` and migration history table DDL.

---

[1.4.0]: https://gitcode.com/rf2026/rust-ef/releases/tag/v1.4.0
[1.1.0]: https://gitcode.com/rf2026/rust-ef/releases/tag/v1.1.0
[1.0.0]: https://gitcode.com/rf2026/rust-ef/releases/tag/v1.0.0
[0.5]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.5
[0.4]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.4
[0.3.5]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.3.5
[0.3]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.3
[0.2]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.2
[0.1]: https://gitcode.com/rf2026/rust-ef/releases/tag/v0.1
