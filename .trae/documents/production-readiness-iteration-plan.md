# REF 框架生产就绪迭代计划

> **范围**: P0 阻断修复 + P1 生产加固
> **目标**: 三库均无阻断性缺陷，可投入生产；运维就绪
> **版本**: v1.3.0 → v1.4.0（本次迭代产物）
> **预估工作量**: 约 1 周
> **决策**: 不重构 `from_row` 的 `Vec<String>` 类型擦除（仅修复 MySQL bug）

---

## 一、当前状态分析

### 已就绪能力（无需改动）
- 架构: type-map DbContext、Scoped 生命周期、Owned Resolution、keyed 多库、实体自动发现
- CRUD: `save_changes()`、ambient + managed 双模式事务（[`ITransaction`](file:///d:/GitCode/RF/rust-ef/crates/core/src/transaction.rs)）、乐观并发、复合主键
- 查询: `linq!` DSL、CTE/Window、IN/NOT IN 子查询、Lazy Loading (opt-in)、全局过滤器
- 迁移: 模型 diff、三方言 SQL、CLI、history 表
- 性能: [`MetadataCache`](file:///d:/GitCode/RF/rust-ef/crates/core/src/metadata_cache.rs) 进程级元数据缓存
- 测试: 302 个 `#[test]`/`#[tokio::test]`

### 阻断性缺陷（P0）

#### P0-1: MySQL Provider 静默数据丢失 bug
- **位置**: [mysql/connection.rs:57-58](file:///d:/GitCode/RF/rust-ef/crates/mysql/src/connection.rs#L57-L58)
- **现状**: `row.try_get::<String, _>(i).unwrap_or_else(|_| "NULL".to_string())`
- **影响**: 数字、bool、bytes、DATETIME 等所有非 String 列**静默变成 "NULL"**，无错误抛出
- **对比**: PostgreSQL 的 [`cell_to_string`](file:///d:/GitCode/RF/rust-ef/crates/postgres/src/connection.rs#L20-L63) 按 OID 精细分发，MySQL 缺失等价实现
- **结论**: MySQL 路径实际不可用于生产

#### P0-2: MetadataCache poison 后 panic
- **位置**: [metadata_cache.rs:62](file:///d:/GitCode/RF/rust-ef/crates/core/src/metadata_cache.rs#L62)
- **现状**: `self.by_key.lock().expect("MetadataCache poisoned")`
- **影响**: 任意 DbContext 在持锁期间 panic，会导致整个应用的 `MetadataCache` 永久 poison，后续所有请求 panic
- **结论**: 单点失败风险，生产不可接受

### 生产加固项（P1）

#### P1-3: SQLite 无连接池
- **位置**: [sqlite/provider.rs:10](file:///d:/GitCode/RF/rust-ef/crates/sqlite/src/provider.rs#L10)
- **现状**: `Arc<Mutex<rusqlite::Connection>>` 单连接（已启用 WAL + busy_timeout=5000ms）
- **影响**: 整个应用共享一个 SQLite 连接，高并发下序列化等待
- **对比**: PG 用 deadpool、MySQL 用 sqlx pool，唯独 SQLite 不对等
- **注**: SQLite 文件模式支持多连接（WAL 已启用），仅 `:memory:` 需特殊处理

#### P1-4: SPEC 文档与实际版本脱节
- **位置**: [docs/PRODUCTION_READINESS_SPEC.md](file:///d:/GitCode/RF/rust-ef/docs/PRODUCTION_READINESS_SPEC.md)
- **现状**: 停留在 v1.1.0，未文档化 v1.3 新功能
- **缺失内容**: MetadataCache、ITransaction、savepoint、isolation level、rust-dix 0.6 迁移、Owned Resolution
- **测试数量**: SPEC 称 278，实际 302

#### P1-5: PG/MySQL 集成测试覆盖不对等
- **现状**: SQLite 9 个端到端测试，PG/MySQL 各仅 1 个测试文件
- **影响**: 三库行为一致性无法保证，MySQL bug 长期未暴露即此原因
- **缺失场景**: 空表查询、IN/聚合/分页、种子数据、事务回滚、复合主键 CRUD、全类型映射

#### P1-6: PostgreSQL 默认 NoTls 硬编码
- **位置**: [postgres/provider.rs:6,33](file:///d:/GitCode/RF/rust-ef/crates/postgres/src/provider.rs#L6)
- **现状**: `use tokio_postgres::NoTls;` 硬编码，`cfg.create_pool(Some(Runtime::Tokio1), NoTls)`
- **影响**: 生产数据库连接明文传输口令，高风险
- **SPEC 标注**: 已列为"部署加固建议"，但未提供启用路径

---

## 二、提议的变更

### P0-1: MySQL `cell_to_string` 按列类型分发

**文件**: `crates/mysql/src/connection.rs`、`crates/mysql/Cargo.toml`

**现状代码** (connection.rs:30-65):
```rust
async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>> {
    // ...
    let result = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    row.try_get::<String, _>(i)
                        .unwrap_or_else(|_| "NULL".to_string())  // ← bug
                })
                .collect()
        })
        .collect();
    Ok(result)
}
```

**修复方案**: 仿照 PG 的 `cell_to_string` 模式，按 `sqlx::mysql::MySqlTypeInfo` 分发：

```rust
fn cell_to_string(row: &sqlx::mysql::MySqlRow, col_idx: usize, type_info: &sqlx::mysql::MySqlTypeInfo) -> String {
    use sqlx::Column;
    // NULL 优先检测
    if row.try_get::<Option<i8>, _>(col_idx).is_ok() {
        // ... 按 type_info.kind() 分发
    }
    match type_info.kind() {
        // 布尔
        sqlx::mysql::MySqlTypeKind::Tiny => {
            row.try_get::<Option<bool>, _>(col_idx)
                .ok().flatten()
                .map(|b| if b { "1".into() } else { "0".into() })
                .unwrap_or_else(|| "NULL".into())
        }
        // 整数族
        sqlx::mysql::MySqlTypeKind::Short | sqlx::mysql::MySqlTypeKind::Long | ... => {
            row.try_get::<Option<i64>, _>(col_idx)
                .ok().flatten()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "NULL".into())
        }
        // 浮点
        sqlx::mysql::MySqlTypeKind::Float | sqlx::mysql::MySqlTypeKind::Double => {
            row.try_get::<Option<f64>, _>(col_idx)
                .ok().flatten()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "NULL".into())
        }
        // 字符串
        sqlx::mysql::MySqlTypeKind::VarChar | sqlx::mysql::MySqlTypeKind::String => {
            row.try_get::<Option<String>, _>(col_idx)
                .ok().flatten()
                .unwrap_or_else(|| "NULL".into())
        }
        // 时间（需 sqlx chrono feature）
        sqlx::mysql::MySqlTypeKind::DateTime => {
            row.try_get::<Option<chrono::NaiveDateTime>, _>(col_idx)
                .ok().flatten()
                .map(|dt| dt.to_string())
                .unwrap_or_else(|| "NULL".into())
        }
        // bytes
        sqlx::mysql::MySqlTypeKind::Blob => {
            row.try_get::<Option<Vec<u8>>, _>(col_idx)
                .ok().flatten()
                .map(|b| format!("{:x}", b.iter().fold(0u64, |acc, &x| acc ^ x as u64)))  // 简化
                .unwrap_or_else(|| "NULL".into())
        }
        _ => row.try_get::<Option<String>, _>(col_idx)
                .ok().flatten()
                .unwrap_or_else(|| "NULL".into()),
    }
}
```

**Cargo.toml 变更**: 确保 `sqlx` 启用 `chrono` feature（若未启用）:
```toml
sqlx = { version = "0.7", features = ["mysql", "chrono", "uuid", "runtime-tokio"] }
```

**测试验证**: 在 `crates/core/tests/mysql_crud_tests.rs` 新增测试覆盖整数/bool/时间列读取，断言值正确非"NULL"。

---

### P0-2: MetadataCache poison 后重建而非 panic

**文件**: `crates/core/src/metadata_cache.rs`

**现状代码** (line 60-69):
```rust
pub fn get_or_build(&self, context_key: Option<&str>) -> Arc<BuiltMetadata> {
    let key = context_key.map(|s| s.to_string());
    let mut cache = self.by_key.lock().expect("MetadataCache poisoned");  // ← panic
    if let Some(built) = cache.get(&key) {
        return Arc::clone(built);
    }
    let built = Arc::new(Self::build(context_key));
    cache.insert(key, Arc::clone(&built));
    built
}
```

**修复方案**: poison 时清空缓存并重建（poison 说明上次构建中断，缓存可能不完整）:

```rust
pub fn get_or_build(&self, context_key: Option<&str>) -> Arc<BuiltMetadata> {
    let key = context_key.map(|s| s.to_string());
    // poison 时获取内部数据并清空，触发重建
    let mut cache = match self.by_key.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // 上次持锁线程 panic，缓存可能不完整，清空重建
            let mut guard = poisoned.into_inner();
            guard.clear();
            guard
        }
    };
    if let Some(built) = cache.get(&key) {
        return Arc::clone(built);
    }
    let built = Arc::new(Self::build(context_key));
    cache.insert(key, Arc::clone(&built));
    built
}
```

**测试验证**: 在 `crates/core/tests/metadata_cache_tests.rs` 新增 poison 重建测试（模拟 poison 后调用 `get_or_build` 应成功）。

---

### P1-3: SQLite 引入 r2d2 连接池

**文件**: `crates/sqlite/Cargo.toml`、`crates/sqlite/src/provider.rs`、`crates/sqlite/src/connection.rs`

**依赖**: 新增 `r2d2 = "0.8"` 和 `r2d2_sqlite = "0.24"`

**现状代码** (provider.rs):
```rust
pub struct SqliteProvider {
    conn: Arc<Mutex<rusqlite::Connection>>,
}
```

**修复方案**: 替换为 r2d2 池:

```rust
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

pub struct SqliteProvider {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteProvider {
    pub fn new(path: impl AsRef<Path>) -> EFResult<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::builder()
            .max_size(8)  // 默认 8 连接
            .connection_customizer(|conn| {
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
                    .map_err(|e| r2d2::Error::from(e))
            })
            .build(manager)
            .map_err(|e| EFError::Connection(format!("SQLite pool failed: {}", e)))?;
        Ok(Self { pool })
    }

    pub fn new_in_memory() -> EFResult<Self> {
        // :memory: 需共享缓存模式才能跨连接
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder()
            .max_size(1)  // in-memory 只能单连接
            .build(manager)
            .map_err(|e| EFError::Connection(format!("SQLite memory pool failed: {}", e)))?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl IDatabaseProvider for SqliteProvider {
    async fn get_connection(&self) -> EFResult<Box<dyn IAsyncConnection>> {
        let conn = self.pool.get()
            .map_err(|e| EFError::Connection(format!("SQLite pool acquire failed: {}", e)))?;
        Ok(Box::new(SqliteConnection::new(conn)))
    }
    // ...
}
```

**connection.rs 变更**: `SqliteConnection` 持有 `r2d2::PooledConnection<SqliteConnectionManager>` 而非 `Arc<Mutex<Connection>>`:

```rust
pub struct SqliteConnection {
    conn: r2d2::PooledConnection<SqliteConnectionManager>,
}

impl SqliteConnection {
    pub(crate) fn new(conn: r2d2::PooledConnection<SqliteConnectionManager>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl IAsyncConnection for SqliteConnection {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64> {
        // 直接使用 self.conn，无需 lock
        let rp = crate::type_conversion::to_rusqlite_params(params);
        let refs: Vec<&dyn rusqlite::types::ToSql> = rp.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();
        self.conn.execute(sql, refs.as_slice())
            .map(|c| c as u64)
            .map_err(|e| EFError::Query(format!("Execution error: {}", e)))
    }
    // query/begin/commit/... 同理，移除 .lock().await
}
```

**注**: `r2d2::PooledConnection` 是 `Send`，但 `rusqlite::Connection` 非 `Sync`。`IAsyncConnection` 要求 `Send`，需验证 `PooledConnection` 满足。若不满足，回退方案：保持 `Arc<Mutex>` 但包装多个连接的 Vec 或使用 `tokio::sync::Semaphore` 限流。

**回退方案**（若 r2d2 不满足 trait bound）: 引入 `tokio::sync::Semaphore` + `Vec<Arc<Mutex<Connection>>>` 手动池化，max_size=8。

**测试验证**: 在 `crates/core/tests/connection_pool_tests.rs` 扩展 SQLite 并发测试，验证多连接并行执行。

---

### P1-4: SPEC 文档同步到 v1.3

**文件**: `docs/PRODUCTION_READINESS_SPEC.md`、`CHANGELOG.md`

**变更内容**:
1. **版本号**: v1.1.0 → v1.4.0（含本次迭代产物）
2. **执行摘要**: 更新为"v1.4.0 已达成（v1.1 查询保真度 + v1.3 元数据缓存/事务扩展 + v1.4 生产加固）"
3. **新增章节 3.10**: v1.3 元数据缓存（MetadataCache 进程级单例、context_key 键控、poison 重建）
4. **新增章节 3.11**: v1.3 事务接口扩展（ITransaction trait、ambient transaction、savepoint、IsolationLevel、`begin_transaction`/`use_transaction` 双模式）
5. **新增章节 3.12**: v1.3 rust-dix 0.6 迁移（ServiceCollection::build 返回 Arc<ServiceProvider>、get_owned 返回 Result）
6. **新增章节 3.13**: v1.4 生产加固（MySQL cell_to_string 修复、SQLite r2d2 池、PG TLS 可配置）
7. **验收矩阵**: 新增 v1.3、v1.4 列
8. **测试数量**: 278 → 302+（含本次新增测试）
9. **已知限制更新**:
   - 移除: MetadataCache poison panic（已修复）
   - 移除: MySQL 静默数据丢失（已修复）
   - 移除: SQLite 无连接池（已修复）
   - 移除: PG 默认 NoTls（已可配置）
   - 保留: linq! 需显式类型、Lazy Loading opt-in、拦截器只读、from_row Vec<String>、CTE raw 模式 PG 占位符
   - 新增: SQLite `:memory:` 模式仍单连接（r2d2 限制）
10. **实现优先级**: v1.4 已完成项移入"已完成"区块

**CHANGELOG.md**: 在 `[Unreleased]` 后新增 `[1.4.0]` 条目，记录本次所有变更。

---

### P1-5: PG/MySQL 集成测试对齐 SQLite

**文件**: `crates/core/tests/postgres_crud_tests.rs`、`crates/core/tests/mysql_crud_tests.rs`、`crates/core/tests/common/mod.rs`

**现状**: PG/MySQL 各仅 1 个测试文件，仅覆盖基本 CRUD 生命周期。

**修复方案**: 参考 `sqlite_crud_tests.rs` 的 9 个场景，扩展 PG/MySQL:

```rust
// postgres_crud_tests.rs / mysql_crud_tests.rs 扩展
#[tokio::test]
async fn test_crud_lifecycle() { /* 已有，保留 */ }

#[tokio::test]
async fn test_empty_table_query() { /* 新增 */ }

#[tokio::test]
async fn test_in_aggregate_pagination() { /* 新增 */ }

#[tokio::test]
async fn test_seed_data() { /* 新增 */ }

#[tokio::test]
async fn test_transaction_rollback() { /* 新增 */ }

#[tokio::test]
async fn test_composite_primary_key_crud() { /* 新增 */ }

#[tokio::test]
async fn test_full_type_mapping() { /* 新增: bool/Option/i32/i64/f64/String/bytes */ }

#[tokio::test]
async fn test_chrono_uuid_types() { /* 新增: feature 门控 */ }

#[tokio::test]
async fn test_concurrency_conflict() { /* 新增: 乐观并发 */ }
```

**共享 helper**: `tests/common/mod.rs` 已有 CRUD 生命周期 helper，扩展为参数化测试函数，接受 `provider: Arc<dyn IDatabaseProvider>` 参数，三库共用。

**CI**: 已有 GitHub Actions matrix（sqlite/postgres/mysql），无需改动。本地运行需 `RUST_EF_PG_URL` / `RUST_EF_MYSQL_URL` 环境变量。

**验证**: `cargo test --features chrono,uuid,decimal --test postgres_crud_tests` 全绿；`cargo test --features chrono,uuid,decimal --test mysql_crud_tests` 全绿。

---

### P1-6: PostgreSQL TLS 可配置

**文件**: `crates/postgres/src/provider.rs`、`crates/postgres/src/di_extension.rs`、`crates/postgres/Cargo.toml`

**依赖**: 新增 `native-tls = "0.2"`、`tokio-postgres-native-tls = "0.5"`、`postgres-native-tls = "0.5"`

**现状代码** (provider.rs:6,33):
```rust
use tokio_postgres::NoTls;
// ...
let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)
```

**修复方案**: 引入 `PgTlsMode` 枚举，`new` 方法接受 TLS 配置:

```rust
use tokio_postgres::TlsConnect;  // trait
use tokio_postgres::NoTls;

pub enum PgTlsMode {
    Disable,                          // NoTls（当前默认，向后兼容）
    Require(native_tls::TlsConnector), // 强制 TLS
}

pub struct PostgresProvider {
    pool: Pool,
}

impl PostgresProvider {
    /// 向后兼容: 默认 NoTls
    pub fn new(connection_string: &str, pool_size: usize) -> EFResult<Self> {
        Self::new_with_tls(connection_string, pool_size, PgTlsMode::Disable)
    }

    pub fn new_with_tls(
        connection_string: &str,
        pool_size: usize,
        tls: PgTlsMode,
    ) -> EFResult<Self> {
        // ... 解析 config ...
        let pool = match tls {
            PgTlsMode::Disable => cfg.create_pool(Some(Runtime::Tokio1), NoTls),
            PgTlsMode::Require(connector) => {
                let tls = tokio_postgres_native_tls::TlsConnector::new(connector);
                cfg.create_pool(Some(Runtime::Tokio1), tls)
            }
        }.map_err(|e| EFError::Connection(format!("Failed to create pool: {}", e)))?;
        Ok(Self { pool })
    }
}
```

**di_extension.rs 变更**: 新增 `use_postgres_tls` 扩展方法:

```rust
pub trait DbContextOptionsBuilderExt {
    fn use_postgres(&mut self, connection_string: &str) -> &mut Self { /* 已有 */ }

    fn use_postgres_with_tls(
        &mut self,
        connection_string: &str,
        pool_size: usize,
        tls: PgTlsMode,
    ) -> &mut Self {
        let factory = move |_: &str| -> EFResult<Arc<dyn IDatabaseProvider>> {
            Ok(Arc::new(PostgresProvider::new_with_tls(connection_string, pool_size, tls.clone())?))
        };
        self.set_provider_factory("postgres", connection_string, Arc::new(factory));
        self
    }
}
```

**注**: `PgTlsMode` 需实现 `Clone`（factory 闭包可能被多次调用）。`native_tls::TlsConnector` 已实现 `Clone`。

**测试验证**: 在 `crates/postgres/tests/` 新增 TLS 测试（需本地 PG 配置 TLS，或用 `#[ignore]` 标注手动运行）。文档示例化 TLS 启用方式。

**文档**: 在 `docs/rust-ef/11-best-practices/security.md` 新增"启用 PostgreSQL TLS"小节。

---

## 三、假设与决策

### 假设
1. **r2d2_sqlite 兼容性**: 假设 `r2d2::PooledConnection<SqliteConnectionManager>` 满足 `Send` 要求。若不满足，回退到 `tokio::sync::Semaphore` + `Vec<Arc<Mutex<Connection>>>` 手动池化方案。
2. **sqlx mysql type_info.kind() 可用**: 假设 `sqlx::mysql::MySqlTypeInfo::kind()` 方法存在且可枚举。若 API 差异，需调整 `cell_to_string` 实现为 `try_get` 多类型尝试链（参考 SQLite connection.rs:46-50 的 `or_else` 链）。
3. **CI 环境变量**: PG/MySQL 测试依赖 `RUST_EF_PG_URL` / `RUST_EF_MYSQL_URL`，CI 已配置 service containers。本地运行需手动设置。
4. **native-tls 跨平台**: Windows/macOS/Linux 均支持 `native-tls`（用系统证书库）。

### 决策
1. **不重构 `from_row`**: 保持 `Vec<Vec<String>>` ABI，仅修复 MySQL `cell_to_string`。原因：用户明确选择，重构需 macro 自动生成 `IFromRow`，工作量大且破坏所有 entity。
2. **SQLite `:memory:` 保持单连接**: r2d2 的 `SqliteConnectionManager::memory()` 每个连接独立内存数据库，跨连接共享需 `Mode::Memory` + `cache=shared`，但 r2d2 不直接支持。`:memory:` 主要用于测试，单连接可接受。
3. **PG TLS 默认仍 NoTls**: 向后兼容，通过新方法 `use_postgres_with_tls` 启用。不强制生产 TLS（避免破坏现有用户）。
4. **不引入结构化错误**: P2 范围，本次不改造 `EFError` 为结构化（SQLSTATE/约束名）。
5. **不引入运维特性**: 连接池监控/慢查询日志/健康检查 hook 属 P2，本次不做。
6. **`builder.rs` 内部 panic 保留**: `having_internal`/`window_internal` 的 panic 是宏内部不变式（字符串来自 linq! 解析，保证有效），不可达。改造为 `EFResult` 需同步更新宏生成代码，工作量大且无实际收益。

---

## 四、实施顺序与依赖

```
P0-1 (MySQL bug) ──────────────────────────────► 可独立实施
P0-2 (MetadataCache poison) ───────────────────► 可独立实施
P1-3 (SQLite r2d2) ────────────────────────────► 可独立实施（需验证 r2d2 trait bound）
P1-5 (PG/MySQL 测试对齐) ──────────────────────► 依赖 P0-1（MySQL bug 修复后才能写测试）
P1-6 (PG TLS) ─────────────────────────────────► 可独立实施
P1-4 (SPEC 文档同步) ──────────────────────────► 依赖 P0/P1 全部完成（文档化最终状态）
```

**推荐执行顺序**:
1. P0-1 → P0-2（阻断修复，优先）
2. P1-3 → P1-6（独立加固项，可并行）
3. P1-5（测试对齐，验证前述修复）
4. P1-4（文档同步，收尾）

---

## 五、验证步骤

### 单元/集成测试
```bash
# P0-1 验证: MySQL bug 修复
cargo test --features chrono,uuid,decimal --test mysql_crud_tests
# 新增测试: test_int_bool_datetime_not_null → 断言非 "NULL"

# P0-2 验证: MetadataCache poison 重建
cargo test --test metadata_cache_tests
# 新增测试: test_poison_recovery → 模拟 poison 后 get_or_build 成功

# P1-3 验证: SQLite 连接池
cargo test --test connection_pool_tests
cargo test --test sqlite_crud_tests
# 新增测试: test_concurrent_connections → 8 并发连接并行执行

# P1-5 验证: PG/MySQL 测试对齐
RUST_EF_PG_URL=postgres://... cargo test --features chrono,uuid,decimal --test postgres_crud_tests
RUST_EF_MYSQL_URL=mysql://... cargo test --features chrono,uuid,decimal --test mysql_crud_tests
# 9 个场景全绿

# P1-6 验证: PG TLS
cargo build -p rust-ef-postgres  # 编译通过
# 手动测试: 配置本地 PG 强制 TLS，用 use_postgres_with_tls 连接
```

### 全量回归
```bash
cargo clippy --workspace --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --all-features --no-fail-fast
cargo bench --workspace --no-run
```

### 预期结果
- 测试数量: 302 → ~320（新增 ~18 个测试: MySQL 8 + PG 8 + metadata_cache 1 + connection_pool 1）
- Clippy 零 warning
- fmt 一致
- 三库集成测试全绿
- SPEC 文档与代码一致（v1.4.0）

---

## 六、交付物清单

| 交付物 | 文件 |
|--------|------|
| MySQL cell_to_string 修复 | `crates/mysql/src/connection.rs` |
| MySQL Cargo.toml feature 启用 | `crates/mysql/Cargo.toml` |
| MetadataCache poison 重建 | `crates/core/src/metadata_cache.rs` |
| SQLite r2d2 连接池 | `crates/sqlite/src/provider.rs`, `connection.rs`, `Cargo.toml` |
| PG TLS 可配置 | `crates/postgres/src/provider.rs`, `di_extension.rs`, `Cargo.toml` |
| PG/MySQL 测试对齐 | `crates/core/tests/postgres_crud_tests.rs`, `mysql_crud_tests.rs`, `common/mod.rs` |
| MetadataCache poison 测试 | `crates/core/tests/metadata_cache_tests.rs` |
| SQLite 连接池测试 | `crates/core/tests/connection_pool_tests.rs` |
| SPEC 文档同步 | `docs/PRODUCTION_READINESS_SPEC.md` |
| CHANGELOG 更新 | `CHANGELOG.md` |
| 安全文档 TLS 章节 | `docs/rust-ef/11-best-practices/security.md` |

---

## 七、风险评估

| 风险 | 概率 | 影响 | 缓解 |
|------|:---:|:---:|------|
| r2d2_sqlite PooledConnection 不满足 Send | 中 | 高 | 回退到 Semaphore + Vec<Mutex> 手动池化 |
| sqlx mysql type_info API 差异 | 低 | 中 | 用 try_get 多类型链替代 kind() 分发 |
| native-tls 跨平台证书问题 | 低 | 中 | 文档说明各平台证书配置 |
| PG/MySQL CI service container 不稳定 | 中 | 低 | 测试加 retry 或本地手动验证 |
| MetadataCache poison 重建掩盖真实 bug | 低 | 低 | 记录日志（tracing::warn）便于诊断 |

---

*本计划基于 2026-07-08 代码状态制定，执行时如遇与计划不符的实际情况，应暂停并重新评估。*
