# 移除 IDbContext trait 实现计划

## Context

`IDbContext` trait 存在系统性架构缺陷：

1. **trait 写方法需要 `&mut self`，但 DI 只返回 `Arc<T>`**：`save_changes(&mut self)`、`set::<T>(&mut self)` 等核心方法在 `Arc<dyn IDbContext>` 上无法调用，导致官方模板 `web-handler-crud.rs`、`di-setup.rs` 无法编译
2. **`set::<T>()` 是泛型方法，无法放入 object-safe trait**：迫使用户依赖具体 `DbContext` 类型
3. **接口导向的代价过高**：要修复需引入内部可变性（RwLock）或 type-erased hack，牺牲 Rust 性能和特性

**用户决策**：移除 `IDbContext`，放弃不完整的接口导向目标，保持 Rust 性能和特性最佳。`DbContext` 作为具体上下文类型直接使用，通过 DI 注册为 `Arc<DbContext>`。

影响范围：23 个 `.rs` 文件 + 27 个 `.md` 文件，但 `IDbContext` 引用全部集中在 `crates/core` 内部，其他 crate 不受影响。

---

## 阶段一：框架核心代码（3 个文件）

### 1.1 `crates/core/src/db_context.rs`

**删除（L518-663）：**
- `IDbContext` trait 定义（L522-534）
- `IDbContextExt` trait 定义（L537-557）
- blanket impl `impl<T: IDbContext + Send + Sync> IDbContextExt for T`（L559-560）
- `impl IDbContext for DbContext`（L566-663）

**新增**：在原 trait 位置添加独立 `impl DbContext` 块，将以下方法迁移为固有方法（去掉 `#[async_trait]`，使用原生 `async fn`）：

| 方法 | 签名 | 来源 |
|------|------|------|
| `provider` | `pub fn provider(&self) -> &dyn IDatabaseProvider` | IDbContext trait |
| `change_tracker` | `pub fn change_tracker(&self) -> &ChangeTracker` | IDbContext trait |
| `change_tracker_mut` | `pub fn change_tracker_mut(&mut self) -> &mut ChangeTracker` | IDbContext trait |
| `begin_transaction` | `pub async fn begin_transaction(&self) -> EFResult<Box<dyn IAsyncConnection>>` | IDbContext 默认方法 |
| `save_changes` | `pub async fn save_changes(&mut self) -> EFResult<SaveChangesResult>` | IDbContext trait（完整方法体原样搬迁，L579-661） |
| `use_transaction` | `pub async fn use_transaction<F, Fut, R>(&self, f: F) -> EFResult<R>` | IDbContextExt trait |

**更新模块文档（L1-41）：**
- 移除 "IDbContext is object-safe" 描述
- `Arc<dyn IDbContext>` → `Arc<DbContext>`
- `scope.get::<dyn IDbContext>()` → `scope.get::<DbContext>()`

### 1.2 `crates/core/src/di.rs`

- L60：`use ...IDbContext` → 移除
- L105、L121：`Arc::new(ctx) as Arc<dyn IDbContext>` → `Arc::new(ctx)`
- 文档注释中所有 `Arc<dyn IDbContext>` → `Arc<DbContext>`

### 1.3 `crates/core/src/lib.rs`

- L52-54：prelude 中移除 `IDbContext`

### 验证点

```bash
cargo check -p rust-ef
```

重点关注：原生 `async fn`（无 `#[async_trait]`）的 `save_changes` 产生的 Future 是否 `Send`。`IAsyncConnection: Send + Sync` 已确认，方法体内所有 async 调用应产生 `Send` Future。

---

## 阶段二：测试 / 示例 / Bench（20 个 .rs 文件）

### 2.1 仅移除 import 的文件（17 个）

这些文件只在 import 中引用 `IDbContext`，移除后方法调用变为 `DbContext` 固有方法，无需 trait 导入：

**测试文件（12 个）：**
- `tests/common/mod.rs`
- `tests/transaction_composite_tests.rs`
- `tests/tracking_consistency_tests.rs`
- `tests/sqlite_crud_tests.rs`
- `tests/query_filter_exec_tests.rs`
- `tests/production_tests.rs`
- `tests/navigation_perf_tests.rs`
- `tests/extended_types_tests.rs`
- `tests/exists_by_id_tests.rs`
- `tests/concurrency_tests.rs`
- `tests/batch_dml_tests.rs`

**Bench 文件（3 个）：**
- `benches/bench_include.rs`
- `benches/bench_insert.rs`
- `benches/bench_query.rs`

**示例文件（3 个）：**
- `examples/soft_delete/src/main.rs`
- `examples/blog/src/main.rs`（L14 是独立 import 行，直接删除整行）
- `examples/audit/src/main.rs`

### 2.2 需修改 import + 类型标注的文件（1 个）

**`tests/scoped_lifecycle_tests.rs`**：
- import：`IDbContext` → `DbContext`
- `Arc<dyn IDbContext>` → `Arc<DbContext>`（6 处）
- `provider.get::<dyn IDbContext>()` → `provider.get::<DbContext>()`

### 验证点

```bash
cargo check --tests --examples --benches
cargo test -p rust-ef --test scoped_lifecycle_tests
```

---

## 阶段三：技能模板文件（2 个文件）

### 3.1 `templates/di-setup.rs`

- `Arc<dyn IDbContext>` → `Arc<DbContext>`（所有处）
- 移除 `interface-oriented` 描述
- 注释说明 `save_changes` 需要 `&mut self`

### 3.2 `templates/web-handler-crud.rs`

- 移除 `use rust_ef::db_context::IDbContext;`
- `ctx: Arc<dyn IDbContext>` → `ctx: Arc<DbContext>`
- 更新注释：说明 handler 需要获取 `&mut DbContext`

### 3.3 回滚 `references/webapp-integration.md` 的临时修改

之前将 `Arc<dyn IDbContext>` 改为 `DbContext` 的修改需要统一：更新所有相关描述，移除关于"为何使用具体 DbContext 而非接口"的注释（因为 IDbContext 已不存在）。

---

## 阶段四：文档更新（27 个 .md 文件）

### 修改模式

1. `Arc<dyn IDbContext>` → `Arc<DbContext>`
2. `dyn IDbContext` → `DbContext`
3. `use ...IDbContext` / `IDbContextExt` → 删除或改为 `DbContext`
4. "object-safe" / "接口导向" → "DbContext 是具体上下文类型，直接使用"
5. `IDbContextExt::use_transaction` → `DbContext::use_transaction`

### 按文件分类

**核心文档（docs/rust-ef/）：**
- `02-quickstart/dbcontext-and-di.md`
- `02-quickstart/auto-registration.md`
- `02-quickstart/INDEX.md`
- `10-di-interceptors/di-registration.md`
- `10-di-interceptors/keyed-databases.md`
- `09-transactions-migrations/manual-transactions.md`
- `09-transactions-migrations/INDEX.md`
- `03-advanced/multi-tenancy-foundation.md`
- `01-introduction/ecosystem-overview.md`
- `01-introduction/what-is-rust-ef.md`
- `01-introduction/who-should-use.md`

**根目录文档：**
- `README.md`
- `crates/core/README.md`
- `CHANGELOG.md`（历史记录中添加标注）
- `docs/PRODUCTION_READINESS_SPEC.md`

**技能文档（.agents/skills/lref/）：**
- `SKILL.md`
- `references/architecture.md`（移除 object-safe/non-object-safe 表格中的 IDbContext/IDbContextExt）
- `references/quickstart.md`
- `references/pitfalls.md`
- `references/advanced.md`
- `references/webapp-integration.md`

**计划文档（.trae/documents/）：** 历史规划文档，在文件顶部添加"IDbContext 相关内容已过时"标注，不逐行修改。

---

## 阶段五：验证

```bash
# 1. 编译
cargo check --workspace
cargo check --tests --examples --benches

# 2. 测试
cargo test --workspace

# 3. Lint
cargo clippy --workspace --all-targets -- -D warnings

# 4. 文档
cargo doc --workspace --no-deps

# 5. 示例运行
cargo run --example blog
cargo run --example soft_delete
```

**重点验证：**
- `scoped_lifecycle_tests`：`Arc<DbContext>` 的 DI 解析和 scope 隔离
- `transaction_composite_tests`：`save_changes` 固有方法的事务行为
- 所有 CRUD 测试：`save_changes()` 作为固有方法正常工作

---

## 实施顺序

1. `db_context.rs`：删除 trait + 添加固有方法 + 更新文档
2. `di.rs`：修改注册逻辑
3. `lib.rs`：修改 prelude
4. `cargo check -p rust-ef`：验证核心编译
5. 17 个 import-only 文件：批量移除 `IDbContext` 导入
6. `scoped_lifecycle_tests.rs`：修改 import + 类型标注
7. `cargo check --tests --examples --benches`：验证全部编译
8. 2 个 skill 模板文件
9. 27 个 .md 文档
10. `cargo test --workspace` + `cargo clippy`：完整验证