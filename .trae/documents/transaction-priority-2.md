# 优先级 2 — 事务接口扩展 (savepoint + 隔离级别 + ambient transaction)

## Summary

为 `IAsyncConnection` trait 添加 savepoint 与隔离级别能力;为 `DbContext` 引入 **ambient transaction**(环境事务)机制,使 `save_changes()` 能够复用外部开启的事务连接,从而让 `use_transaction` 与 `save_changes` 在同一事务内协作,并解决 `&self` vs `&mut self` 的签名不一致问题。设计对齐 EFCore 的 `Database.BeginTransaction()` / `Transaction.Commit()` 模式。

---

## Current State Analysis

### IAsyncConnection trait (crates/core/src/provider.rs:534-547)

仅有 5 个方法,无 savepoint、无隔离级别:

```rust
#[async_trait]
pub trait IAsyncConnection: Send + Sync {
    async fn execute(&mut self, sql: &str, params: &[DbValue]) -> EFResult<u64>;
    async fn query(&mut self, sql: &str, params: &[DbValue]) -> EFResult<Vec<Vec<String>>>;
    async fn begin_transaction(&mut self) -> EFResult<()>;
    async fn commit_transaction(&mut self) -> EFResult<()>;
    async fn rollback_transaction(&mut self) -> EFResult<()>;
}
```

### 三 provider 实现

| Provider | 文件 | 底层类型 | begin/commit/rollback SQL |
|---|---|---|---|
| SQLite | `crates/sqlite/src/connection.rs` | `Arc<Mutex<rusqlite::Connection>>` (共享) | `BEGIN TRANSACTION` / `COMMIT` / `ROLLBACK` |
| PostgreSQL | `crates/postgres/src/connection.rs` | `deadpool_postgres::Client` (池化) | `BEGIN` / `COMMIT` / `ROLLBACK` (via `simple_query`) |
| MySQL | `crates/mysql/src/connection.rs` | `sqlx::pool::PoolConnection<sqlx::MySql>` (池化) | `START TRANSACTION` / `COMMIT` / `ROLLBACK` |

**关键差异**:SQLite 所有 `get_connection()` 返回的 wrapper 共享同一 `Arc<Mutex<Connection>>`;PG/MySQL 每次返回池中不同物理连接。因此当前 `save_changes()` 与 `use_transaction()` 在 PG/MySQL 下根本无法共享事务(各自从池取不同连接)。

### DbContext 事务路径 (crates/core/src/db_context.rs)

**`begin_transaction(&self)` (L537-541)** — 返回裸 `Box<dyn IAsyncConnection>`:
- 无 ambient 概念,返回后 DbContext 自身不持有该连接
- `save_changes()` 之后调用会从池取**新**连接,与该事务无关

**`save_changes(&mut self)` (L547-631)**:
- L572: `let mut conn = self.provider.get_connection().await?;` — 每次取新连接
- L573: `conn.begin_transaction().await?;` — 自管事务
- 遍历所有 DbSet,`saver.save(&mut *conn, ...)` 共享同一连接
- L603: `conn.commit_transaction().await?` 或 L592 rollback
- 无法注入外部事务

**`use_transaction<F, Fut, R>(&self, f)` (L636-654)**:
- 签名 `&self`(非 `&mut self`)— 与 `save_changes` 的 `&mut self` 不一致
- 取新连接、开事务、把 `&mut dyn IAsyncConnection` 给闭包
- 完全绕过 ChangeTracker
- 无法与 `save_changes()` 共享连接(PG/MySQL 取不同池连接)

### 现有测试

- `transaction_composite_tests.rs`:仅测试 `save_changes` 的 commit/rollback 原子性,**无 savepoint/隔离级别测试,无 `use_transaction` 调用**
- `sqlite_crud_tests.rs`:多处调用 `conn.begin_transaction()`(trait 方法,在裸 `Box<dyn IAsyncConnection>` 上),**不调用 `DbContext::begin_transaction()`**
- 全代码库无 `SAVEPOINT` / `isolation` / `IsolationLevel` 匹配

### 已有错误类型

`EFError::Transaction(String)` (error.rs:42) 已存在,savepoint/隔离级别错误可复用。

---

## Proposed Changes

### 1. 扩展 `IAsyncConnection` trait + 新增 `IsolationLevel` 枚举

**文件**: `crates/core/src/provider.rs`

在 `IAsyncConnection` trait 之前新增枚举:

```rust
/// ANSI SQL 事务隔离级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}
```

在 `IAsyncConnection` trait 中**新增 4 个必需方法**(无默认实现 — SQL 方言差异):

```rust
#[async_trait]
pub trait IAsyncConnection: Send + Sync {
    // ... 现有 5 个方法保持不变 ...

    /// 在当前事务内创建一个 savepoint。
    async fn create_savepoint(&mut self, name: &str) -> EFResult<()>;
    /// 释放(提交)一个已创建的 savepoint,清除其回滚点。
    async fn release_savepoint(&mut self, name: &str) -> EFResult<()>;
    /// 回滚到指定 savepoint,保留外层事务。
    async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()>;
    /// 设置当前事务的隔离级别(必须在 `begin_transaction` 之后调用)。
    async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()>;
}
```

**为何无默认实现**:SQLite 用 `RELEASE name`,MySQL 用 `RELEASE SAVEPOINT name`,PostgreSQL 两者皆可;`set_transaction_isolation` 在 SQLite 是 `PRAGMA read_uncommitted`,在 PG/MySQL 是 `SET TRANSACTION ISOLATION LEVEL ...`。方言差异使默认实现不可行。

### 2. SQLite provider 实现

**文件**: `crates/sqlite/src/connection.rs`

在 `impl IAsyncConnection for SqliteConnection` 块内追加:

```rust
async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.execute(&format!("SAVEPOINT {}", name), &[]).await.map(|_| ())
}
async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.execute(&format!("RELEASE {}", name), &[]).await.map(|_| ())
}
async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.execute(&format!("ROLLBACK TO {}", name), &[]).await.map(|_| ())
}
async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
    // SQLite 仅支持 ReadUncommitted vs Serializable(默认)。
    // ReadUncommitted 通过 PRAGMA read_uncommitted=ON 启用;其余级别强制回 SERIALIZABLE。
    let sql = match level {
        IsolationLevel::ReadUncommitted => "PRAGMA read_uncommitted = ON",
        _ => "PRAGMA read_uncommitted = OFF",
    };
    self.execute(sql, &[]).await.map(|_| ())
}
```

**注意**:SQLite 的 savepoint 语法是 `RELEASE name`(无 `SAVEPOINT` 关键字)、`ROLLBACK TO name`(无 `SAVEPOINT` 关键字)。参考 https://www.sqlite.org/lang_savepoint.html。

### 3. PostgreSQL provider 实现

**文件**: `crates/postgres/src/connection.rs`

```rust
async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.client
        .simple_query(&format!("SAVEPOINT {}", name))
        .await
        .map_err(|e| EFError::Transaction(format!("SAVEPOINT failed: {}", e)))?;
    Ok(())
}
async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.client
        .simple_query(&format!("RELEASE SAVEPOINT {}", name))
        .await
        .map_err(|e| EFError::Transaction(format!("RELEASE failed: {}", e)))?;
    Ok(())
}
async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.client
        .simple_query(&format!("ROLLBACK TO SAVEPOINT {}", name))
        .await
        .map_err(|e| EFError::Transaction(format!("ROLLBACK TO failed: {}", e)))?;
    Ok(())
}
async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
    let sql = format!(
        "SET TRANSACTION ISOLATION LEVEL {}",
        match level {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    );
    self.client
        .simple_query(&sql)
        .await
        .map_err(|e| EFError::Transaction(format!("SET ISOLATION failed: {}", e)))?;
    Ok(())
}
```

**注意**:`SET TRANSACTION ISOLATION LEVEL` 在 PG 中必须在 `BEGIN` 之后、任何查询之前执行。

### 4. MySQL provider 实现

**文件**: `crates/mysql/src/connection.rs`

```rust
async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.execute(&format!("SAVEPOINT {}", name), &[]).await.map(|_| ())
}
async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.execute(&format!("RELEASE SAVEPOINT {}", name), &[]).await.map(|_| ())
}
async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
    self.execute(&format!("ROLLBACK TO SAVEPOINT {}", name), &[]).await.map(|_| ())
}
async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
    let sql = format!(
        "SET TRANSACTION ISOLATION LEVEL {}",
        match level {
            IsolationLevel::ReadUncommitted => "READ UNCOMMITTED",
            IsolationLevel::ReadCommitted => "READ COMMITTED",
            IsolationLevel::RepeatableRead => "REPEATABLE READ",
            IsolationLevel::Serializable => "SERIALIZABLE",
        }
    );
    self.execute(&sql, &[]).await.map(|_| ())
}
```

### 5. DbContext ambient transaction 机制

**文件**: `crates/core/src/db_context.rs`

#### 5.1 新增字段

在 `DbContext` struct (L329-338) 添加:

```rust
pub struct DbContext {
    sets: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    savers: HashMap<TypeId, Box<dyn ErasedSetOps>>,
    entity_metas: HashMap<TypeId, EntityTypeMeta>,
    model_builder: ModelBuilder,
    change_tracker: ChangeTracker,
    provider: Arc<dyn IDatabaseProvider>,
    interceptor_pipeline: InterceptorPipeline,
    lazy_loading_enabled: bool,
    /// 环境事务:由 `begin_transaction()` 开启,`commit_transaction()` /
    /// `rollback_transaction()` 关闭。存在时,`save_changes()` 复用此连接且
    /// 不自行 begin/commit/rollback。`take()` 模式避免与 `&mut self` 借用冲突。
    ambient_transaction: Option<Box<dyn IAsyncConnection>>,
}
```

在 `from_options()` (L348-364) 初始化:`ambient_transaction: None,`

#### 5.2 **BREAKING**:重定义 `begin_transaction`

替换现有 L536-541:

```rust
/// 开启一个环境事务。后续的 `save_changes()` 调用将复用此事务的连接,
/// 不自行 begin/commit/rollback。必须配对调用 `commit_transaction()` 或
/// `rollback_transaction()`。
///
/// 重复调用(前一事务未关闭)返回 `EFError::Transaction`。
pub async fn begin_transaction(&mut self) -> EFResult<()> {
    if self.ambient_transaction.is_some() {
        return Err(EFError::Transaction(
            "ambient transaction already active; commit or rollback first".into(),
        ));
    }
    let mut conn = self.provider.get_connection().await?;
    conn.begin_transaction().await?;
    self.ambient_transaction = Some(conn);
    Ok(())
}

/// 提交当前环境事务并清除。无环境事务时返回错误。
pub async fn commit_transaction(&mut self) -> EFResult<()> {
    let mut conn = self
        .ambient_transaction
        .take()
        .ok_or_else(|| EFError::Transaction("no ambient transaction to commit".into()))?;
    conn.commit_transaction().await
}

/// 回滚当前环境事务并清除。无环境事务时返回错误。
pub async fn rollback_transaction(&mut self) -> EFResult<()> {
    let mut conn = self
        .ambient_transaction
        .take()
        .ok_or_else(|| EFError::Transaction("no ambient transaction to rollback".into()))?;
    conn.rollback_transaction().await
}
```

**影响分析**:`DbContext::begin_transaction(&self) -> Box<dyn IAsyncConnection>` 当前无测试/示例调用(已 grep 验证)。旧签名返回裸连接,与新 ambient 语义不兼容,直接替换。`IAsyncConnection::begin_transaction()` trait 方法不变,`sqlite_crud_tests.rs` 中 `conn.begin_transaction()` 调用不受影响。

#### 5.3 修改 `save_changes` 复用 ambient 事务

替换 L547-631 的事务管理部分。核心改动:**用 `take()` / restore 模式从 `self.ambient_transaction` 取出连接,避免与 `&mut self.sets` 借用冲突**。

```rust
pub async fn save_changes(&mut self) -> EFResult<SaveChangesResult> {
    // detect_changes + build configured_metas + on_saving (L548-570 保持不变)
    // ...

    // === 事务连接获取 ===
    // 若有 ambient_transaction,取出复用(不 begin/commit/rollback);
    // 否则自管事务(原行为)。
    let ambient = self.ambient_transaction.take();
    let mut conn: Box<dyn IAsyncConnection> = match ambient {
        Some(c) => c,
        None => {
            let mut c = self.provider.get_connection().await?;
            c.begin_transaction().await?;
            c
        }
    };
    let is_ambient = self.ambient_transaction_is_some; // 见下方说明

    // === 遍历 DbSet 保存(L575-602 改用 &mut *conn)==
    let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
    let mut total_added = 0usize;
    // ... 略 ...
    for type_id in &type_ids {
        let saver = self.savers.get(type_id).expect("saver not registered");
        let set = self.sets.get_mut(type_id).unwrap();
        let meta = configured_metas.get(type_id)
            .or_else(|| self.entity_metas.get(type_id))
            .expect("meta not found");
        let (a, u, d) = match saver.save(&mut *conn, &*self.provider, set.as_mut(), meta).await {
            Ok(r) => r,
            Err(e) => {
                if !is_ambient {
                    let _ = conn.rollback_transaction().await;
                }
                // restore ambient so user can rollback
                if is_ambient { self.ambient_transaction = Some(conn); }
                self.interceptor_pipeline.on_save_failed(&save_ctx, &e).await;
                return Err(e);
            }
        };
        total_added += a;
        total_updated += u;
        total_deleted += d;
    }

    if is_ambient {
        // 交还连接给 ambient,不 commit
        self.ambient_transaction = Some(conn);
    } else {
        if let Err(e) = conn.commit_transaction().await {
            self.interceptor_pipeline.on_save_failed(&save_ctx, &e).await;
            return Err(e);
        }
    }

    // accept_all_changes + clear + on_saved (L609-624 保持不变)
    // ...
    Ok(SaveChangesResult { added: total_added, updated: total_updated, deleted: total_deleted })
}
```

**避免 `is_ambient` 标志的更简洁写法**:在取连接时用一个枚举标记来源:

```rust
enum TxnSource { Ambient(Box<dyn IAsyncConnection>), Managed(Box<dyn IAsyncConnection>) }
```

实现时优先用此模式,逻辑更清晰(避免布尔标志误用)。

#### 5.4 新增 `use_transaction_scope`(ambient + `save_changes` 整合)

**保留**现有 `use_transaction(&self, f: FnOnce(&mut dyn IAsyncConnection))` — 它服务"纯原始 SQL 事务"场景,不涉及 ChangeTracker,签名 `&self` 适合只读上下文。**新增** `use_transaction_scope` 服务"ORM + 原始 SQL 混合事务"场景:

```rust
/// 在一个环境事务内执行闭包。闭包内可调用 `self.save_changes()`(复用事务连接)、
/// `self.create_savepoint()`、原始 SQL 等。成功提交,失败回滚。
///
/// 与 `use_transaction` 的区别:闭包接收 `&mut DbContext` 而非裸连接,
/// `save_changes()` 内部会复用本事务的连接,实现真正的共享事务。
pub async fn use_transaction_scope<F, Fut, R>(&mut self, f: F) -> EFResult<R>
where
    F: FnOnce(&mut Self) -> Fut + Send,
    Fut: Future<Output = EFResult<R>> + Send,
    R: Send,
{
    self.begin_transaction().await?;
    match f(self).await {
        Ok(r) => {
            self.commit_transaction().await?;
            Ok(r)
        }
        Err(e) => {
            let _ = self.rollback_transaction().await;
            Err(e)
        }
    }
}
```

**借用分析**:`begin_transaction(&mut self)` 借用结束后,`f(self)` 重新借用 `&mut Self`,闭包内 `self.save_changes()` 再次 `&mut self` — 全部顺序借用,无并发冲突。`save_changes()` 内部用 `take()` 取出 ambient 连接,避开与 `self.sets` 的可变借用冲突。

#### 5.5 DbContext 级 savepoint/isolation 代理方法

为易用性,在 `DbContext` 上提供代理方法(操作 ambient 事务):

```rust
pub async fn create_savepoint(&mut self, name: &str) -> EFResult<()> {
    let conn = self.ambient_transaction.as_mut()
        .ok_or_else(|| EFError::Transaction("create_savepoint requires an active ambient transaction".into()))?;
    conn.create_savepoint(name).await
}
pub async fn release_savepoint(&mut self, name: &str) -> EFResult<()> {
    let conn = self.ambient_transaction.as_mut()
        .ok_or_else(|| EFError::Transaction("release_savepoint requires an active ambient transaction".into()))?;
    conn.release_savepoint(name).await
}
pub async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()> {
    let conn = self.ambient_transaction.as_mut()
        .ok_or_else(|| EFError::Transaction("rollback_to_savepoint requires an active ambient transaction".into()))?;
    conn.rollback_to_savepoint(name).await
}
pub async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
    let conn = self.ambient_transaction.as_mut()
        .ok_or_else(|| EFError::Transaction("set_transaction_isolation requires an active ambient transaction".into()))?;
    conn.set_transaction_isolation(level).await
}
```

**为何无 ambient 时返回错误而非隐式开启**:隔离级别/savepoint 仅在事务内有意义;隐式开启事务会掩盖用户意图。对齐 EFCore:`CreateSavepoint` 要求事务已开启。

### 6. 导出新类型

**文件**: `crates/core/src/lib.rs` — `prelude` 模块新增:

```rust
pub use crate::provider::IsolationLevel;
```

(可选:也可导出 `IAsyncConnection`,但它已是 `pub trait` 在 `pub mod provider` 中,无需额外导出。)

### 7. 新增集成测试

**文件**: `crates/core/tests/transaction_ext_tests.rs` (新建)

测试矩阵(全部基于 SQLite in-memory,因 PG/MySQL 需外部环境):

1. **`test_ambient_transaction_save_changes_uses_ambient`** — `begin_transaction()` → `set.add()` → `save_changes()` → `commit_transaction()`,验证数据持久化且无嵌套事务错误
2. **`test_ambient_transaction_rollback_undoes_save_changes`** — `begin_transaction()` → `save_changes()` → `rollback_transaction()`,验证数据未持久化
3. **`test_use_transaction_scope_integrates_save_changes`** — `use_transaction_scope(|ctx| async { ctx.set::<T>().add(...); ctx.save_changes().await?; Ok(()) })`,验证 commit 后数据存在
4. **`test_use_transaction_scope_rolls_back_on_error`** — 闭包返回 Err,验证 `save_changes` 的写入被回滚
5. **`test_create_savepoint_and_rollback_to`** — `begin_transaction()` → `save_changes()`(成功) → `create_savepoint("sp1")` → `save_changes()`(再写) → `rollback_to_savepoint("sp1")` → `commit_transaction()`,验证 sp1 后的写入被回滚、sp1 前的保留
6. **`test_release_savepoint`** — 创建 savepoint、release、再尝试 rollback_to 应失败(SQLite 会报错)
7. **`test_savepoint_without_transaction_errors`** — 无 ambient 时调 `create_savepoint` 返回 `EFError::Transaction`
8. **`test_set_isolation_level_serializable`** — `begin_transaction()` → `set_isolation_level(Serializable)` → 提交,验证不报错(SQLite 接受 `PRAGMA read_uncommitted = OFF`)
9. **`test_nested_begin_transaction_errors`** — 已有 ambient 时再次 `begin_transaction()` 返回错误
10. **`test_commit_without_transaction_errors`** — 无 ambient 时 `commit_transaction()` 返回错误

实体定义复用 `transaction_composite_tests.rs` 的 `GoodItem` 模式(独立文件,避免跨文件依赖)。

### 8. 更新 CHANGELOG

**文件**: `CHANGELOG.md` — 在现有 `[Unreleased]` 段落追加 `### Added — Transaction interface extension (priority 2)` 小节,记录:
- `IsolationLevel` 枚举
- `IAsyncConnection` 新增 4 方法(savepoint ×3 + isolation)
- `DbContext` ambient transaction 机制
- **Breaking**: `DbContext::begin_transaction` 签名变更
- 新增 `use_transaction_scope`、`commit_transaction`、`rollback_transaction`、`create_savepoint`、`release_savepoint`、`rollback_to_savepoint`、`set_transaction_isolation`

---

## Assumptions & Decisions

### 决策表

| # | 决策 | 理由 |
|---|---|---|
| D1 | `IsolationLevel` 为 4 变体枚举(ReadUncommitted/ReadCommitted/RepeatableRead/Serializable) | ANSI SQL 标准,EFCore 同款 |
| D2 | savepoint/isolation 方法为 `IAsyncConnection` 必需方法(无默认实现) | SQL 方言差异:SQLite `RELEASE name` vs MySQL `RELEASE SAVEPOINT name`;SQLite `PRAGMA` vs PG/MySQL `SET TRANSACTION` |
| D3 | **Breaking**: `DbContext::begin_transaction(&self) -> Box<dyn IAsyncConnection>` → `begin_transaction(&mut self) -> EFResult<()>` | 旧签名返回裸连接,与 ambient 事务语义不兼容;grep 验证无外部调用;用户记忆"无兼容性包袱" |
| D4 | 保留 `use_transaction(&self, f: FnOnce(&mut dyn IAsyncConnection))` 不变 | 服务"纯原始 SQL 事务"场景,与 `use_transaction_scope` 互补,非冗余 |
| D5 | 新增 `use_transaction_scope(&mut self, f: FnOnce(&mut Self))` 而非改造 `use_transaction` | 不同抽象层级(裸连接 vs DbContext),签名差异(`&self` vs `&mut self`),各有用途 |
| D6 | ambient 连接用 `Option<Box<dyn IAsyncConnection>>` + `take()`/restore 模式 | 避免与 `&mut self.sets` 的可变借用冲突;`Box` 在堆上,`take()` 是 O(1) 指针移动 |
| D7 | DbContext 代理 savepoint/isolation 方法在无 ambient 时返回错误(非隐式开启) | 隔离级别/savepoint 仅在事务内有效;隐式开启会掩盖用户意图;对齐 EFCore |
| D8 | SQLite `set_transaction_isolation` 仅区分 ReadUncommitted vs 其余(强制 Serializable) | SQLite 实际只支持这两档,其余级别在 SQLite 无意义;`PRAGMA read_uncommitted` 是唯一可调旋钮 |
| D9 | 测试仅覆盖 SQLite(in-memory) | PG/MySQL 测试需外部数据库环境;方言正确性靠代码审查 + 类型系统保证;PG/MySQL 的 savepoint SQL 是标准语法 |

### 假设

- 用户接受 `DbContext::begin_transaction` 签名破坏性变更(已验证无调用点)
- `EFError::Transaction(String)` 足够承载新错误场景,无需新增变体
- SQLite in-memory (`:memory:`) 支持 savepoint(已验证:SQLite 3.6+ 支持,`rusqlite` 透传)
- 用户不需要 `IDbContextTransaction` 风格的独立事务对象(方法直接在 DbContext 上更简洁,对齐"极简易用"偏好)

---

## Verification

### 编译验证
1. `cargo build -p rust-ef` — core 编译通过
2. `cargo build -p rust-ef-sqlite` — SQLite provider 编译通过(实现 4 个新 trait 方法)
3. `cargo build -p rust-ef-postgres` — PG provider 编译通过
4. `cargo build -p rust-ef-mysql` — MySQL provider 编译通过
5. `cargo build --workspace` — 全工作区编译通过

### 测试验证
6. `cargo test -p rust-ef --test transaction_composite_tests` — 现有事务测试全通过(回归)
7. `cargo test -p rust-ef --test transaction_ext_tests` — 新增 10 个测试全通过
8. `cargo test -p rust-ef --test sqlite_crud_tests` — 现有 CRUD 测试全通过(回归,验证 `IAsyncConnection::begin_transaction` trait 方法未受影响)
9. `cargo test -p rust-ef` — core 全部测试通过
10. `cargo test -p rust-ef-sqlite` — SQLite provider 测试通过

### 功能验证点
- ambient 事务 + `save_changes()` 共享连接(测试 1)
- ambient 事务 rollback 撤销 `save_changes` 写入(测试 2)
- `use_transaction_scope` 内 `save_changes` 复用事务(测试 3)
- savepoint 创建 + 回滚到 savepoint(测试 5)
- 无 ambient 时调用 savepoint 方法返回错误(测试 7)
- 嵌套 `begin_transaction` 返回错误(测试 9)

---

## 实现顺序

1. **core/provider.rs** — 新增 `IsolationLevel` + 4 trait 方法(必需方法,先编译会失败)
2. **sqlite/connection.rs** — 实现 4 方法(SQLite,用于测试)
3. **postgres/connection.rs** — 实现 4 方法
4. **mysql/connection.rs** — 实现 4 方法
5. **core/db_context.rs** — ambient 字段 + `from_options` 初始化 + 重定义 `begin_transaction` + 新增 `commit/rollback_transaction` + 改造 `save_changes` + 新增 `use_transaction_scope` + 代理方法
6. **core/lib.rs** — prelude 导出 `IsolationLevel`
7. **core/tests/transaction_ext_tests.rs** — 新建测试文件
8. **CHANGELOG.md** — 追加 priority 2 段落
9. 运行验证步骤 1-10
