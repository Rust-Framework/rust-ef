# rust-ef 清理迭代计划 v2（按优先级）

> **目标**：清理所有冗余、无效、违反设计原则和项目开发规范的内容。全部 `.rs` 文件 ≤500 行；所有 `mod.rs` 仅含 `mod` 声明与 `pub use` 重导出（禁止业务代码）；生产代码 panic 路径收敛到 `EFError`；错误体系可程序化区分。
>
> **范围**：仅清理重构，不新增功能。外部 API（`prelude` 导出、公开 trait 签名）保持不变。
>
> **本计划取代 v1**：v1 计划已部分执行（迭代 2.1 完成），本版本基于 2026-07-09 实测现状重新编排。

---

## 一、现状分析（2026-07-09 实测）

### 1.1 阻塞性问题（P0 — 编译错误）

| 问题 | 现状 | 影响 |
|------|------|------|
| **`migration.rs` + `migration/` 并存** | `migration.rs`（1569 行）与 `migration/` 目录（10 子文件，均已创建）**同时存在** | Rust 编译错误：`file for module 'migration' found at both ...migration.rs and ...migration/mod.rs` |

**修复方式**：删除 `crates/core/src/migration.rs`。子文件已就绪且均 ≤500 行（最大 `engine_sql.rs` 366 行）。

### 1.2 `mod.rs` 合规性违规（P0 — 违反项目硬约束）

项目规范要求：**所有 `mod.rs` 仅含 `mod` 声明和 `pub use` 重导出，禁止业务代码**。

| 文件 | 行数 | 违规内容 |
|------|------|----------|
| `crates/core/src/db_context/mod.rs` | 328 | `DbContext` 结构体（9 字段）+ 2 个 `impl DbContext` 块（from_options/set/model/ensure_created/save_changes 等） |
| `crates/core/tests/common/mod.rs` | 401 | `TestItem` 结构体 + `IEntityType`/`IFromRow`/`IGetKeyValues`/`IEntitySnapshot`/`INavigationSetter` 实现 + setup 辅助函数 |

**合规的 mod.rs**（无需修改）：`query/mod.rs`(27)、`migration/mod.rs`(25)、`observability/mod.rs`(23)、`macros/linq/mod.rs`(11)。

### 1.3 超长文件清单（>500 行，共 11 个）

| 文件 | 行数 | 优先级 | 备注 |
|------|------|--------|------|
| `crates/core/src/migration.rs` | 1569 | P0 | 删除即可（已拆分） |
| `crates/core/src/query/builder.rs` | 1219 | P0 | 68 个方法，需拆 6 文件 |
| `crates/macros/src/entity.rs` | 979 | P1 | EntityType derive 宏 |
| `crates/macros/src/linq/parse.rs` | 963 | P1 | LINQ DSL 解析器 |
| `crates/macros/src/linq/compile.rs` | 798 | P1 | LINQ DSL 编译器 |
| `crates/core/src/provider.rs` | 731 | P0 | DbValue + traits |
| `crates/core/src/change_executor.rs` | 728 | P0 | ChangeExecutor |
| `crates/core/tests/cascade_save_tests.rs` | 689 | P1 | 测试 |
| `crates/core/tests/navigation_perf_tests.rs` | 687 | P1 | 测试 |
| `crates/core/tests/having_pagination_dialect_tests.rs` | 554 | P1 | 测试 |
| `crates/core/src/query/ast.rs` | 518 | P0 | 超 18 行 |

### 1.4 代码质量问题

- **生产代码 panic**：66 个 `.unwrap()/.expect()/panic!/unreachable!`，分布在 10 个文件。重点：`save_pipeline.rs`(30)、`di.rs`(10)、`save_phases.rs`(8)、`set_ops.rs`(4)、`query/builder.rs`(4)、`metadata_cache.rs`(3)、`transaction.rs`(3)、`db_context/mod.rs`(2)、`navigation_loader.rs`(1)、`query/state.rs`(1)。
- **`#[allow(dead_code)]`**：9 处（`macros/linq/ast.rs` 1 处、`tests/fk_on_delete_tests.rs` 8 处）。
- **`MetadataCache`**：用 `Mutex<HashMap>`，读多写少应改 `RwLock`。
- **`EFError`**：12 变体枚举，无错误码，调用方无法程序化区分子类型。
- **当前版本**：`1.5.3`（workspace + 5 crate）。

### 1.5 已完成工作

- **v1.5.3（迭代 1）**：MySQL tracing 构建修复、死代码目录删除（1298 行）、CI 硬化、clippy 修复、格式化统一。
- **迭代 2.1（已完成）**：`db_context.rs`（1730 行）→ `db_context/` 目录（5 文件，均 ≤500 行）。但 `mod.rs` 含业务代码，需在迭代 2 修复。

---

## 二、迭代计划

### 迭代 2：修复阻塞 + mod.rs 合规（P0）

#### 2.1 删除 `migration.rs`（恢复编译）

**步骤**：
1. 删除 `crates/core/src/migration.rs`（1569 行原文件）
2. 验证：`cargo check --workspace --all-features` 通过

`migration/mod.rs` 已就绪（25 行，纯声明 + 重导出）。10 个子文件均 ≤366 行。

#### 2.2 修复 `db_context/mod.rs` 合规性

将 `DbContext` 结构体 + impl 块从 `mod.rs` 提取到 `context.rs`：

| 目标文件 | 内容 | 行数预算 |
|----------|------|----------|
| `db_context/mod.rs` | 仅 `mod` 声明 + `pub use` 重导出 | ~30 行 |
| `db_context/context.rs` | `DbContext` 结构体（9 字段 `pub(crate)`）+ 非 save 方法 impl（from_options/set/model/model_builder/discover_entities/detect_changes/ensure_created/ensure_deleted/provider/change_tracker/sql_query/transaction_mut/begin_transaction/use_transaction） | ~280 行 |

**注意**：`save_changes` 在 `save_pipeline.rs`，`drain_cascade_*` 在 `save_phases.rs`（跨文件 impl，Rust 允许）。`context.rs` 的 impl 块用 `impl super::DbContext`。

#### 2.3 修复 `tests/common/mod.rs` 合规性

将测试 fixture 从 `mod.rs` 提取到独立文件：

| 目标文件 | 内容 | 行数预算 |
|----------|------|----------|
| `tests/common/mod.rs` | 仅 `mod` 声明 + `pub use` 重导出 | ~15 行 |
| `tests/common/test_item.rs` | `TestItem` 结构体 + `IEntityType`/`IFromRow`/`IGetKeyValues`/`IEntitySnapshot`/`INavigationSetter` 实现 | ~250 行 |
| `tests/common/setup.rs` | setup 辅助函数（create_provider、ensure_schema 等） | ~150 行 |

**迭代 2 验证检查点**：
- [ ] `migration.rs` 已删除，`cargo check --workspace --all-features` 通过
- [ ] `db_context/mod.rs` ≤30 行（纯声明 + 重导出）
- [ ] `tests/common/mod.rs` ≤15 行（纯声明 + 重导出）
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` 通过
- [ ] `cargo test --workspace --all-features -- --skip postgres --skip mysql` 通过
- [ ] `cargo fmt --all --check` 通过

---

### 迭代 3：core 剩余大文件拆分（P0）

#### 3.1 拆分 `query/builder.rs`（1219 行 → 6 文件）

68 个方法，按功能聚合。`query/builder.rs` → `query/builder/` 目录转换，`query/mod.rs` 中 `mod builder;` 无需修改。

| 目标文件 | 方法 | 行数预算 |
|----------|------|----------|
| `builder/mod.rs` | mod 声明 + `pub use builder_core::QueryBuilder` + `pub use terminal::SelectQueryBuilder` | ~20 行 |
| `builder/core.rs` | `QueryBuilder<T>` 结构体（字段 `pub(super)`）+ `new`/`with_provider`/`with_filter_map`/`with_lazy_loading`/`state`/`filter`/`apply_query_filter` | ~120 行 |
| `builder/filter.rs` | `filter_column`/`filter_not`/`filter_in`/`filter_not_in`/`filter_is_null`/`filter_is_not_null`/`filter_between`/`filter_like`/`filter_not_like`/`order_by_column`/`order_by_desc_column`/`distinct`/`or_where`/`where_exists_internal`/`where_in_subquery_internal`/`skip`/`take` | ~280 行 |
| `builder/join.rs` | `include_internal`/`then_include_internal`/`inner_join_internal`/`left_join_internal`/`right_join_internal`/`full_join_internal`/`cross_join_internal` | ~180 行 |
| `builder/aggregate.rs` | `group_by_internal`/`having_internal`/`having_expr_internal`/`window_internal`/`with_cte_internal`/`with_cte_typed`/`with_recursive_cte_typed`/`from_cte`/`union_*`/`intersect_internal`/`except_internal`/`sum_internal`/`avg_internal`/`min_internal`/`max_internal`/`select_internal` | ~350 行 |
| `builder/terminal.rs` | `to_sql`/`compile_sql`/`compile_state_sql`/`to_list`/`to_list_with_includes`/`first`/`first_or_default`/`count`/`any`/`last`/`last_or_default`/`single`/`single_or_default`/`long_count`/`all`/`contains`/`to_dictionary`/`execute_update`/`execute_delete`/`find`/`find_by_key`/`exists_by_id`/`exists_by_key` + `SelectQueryBuilder` | ~400 行 |

#### 3.2 拆分 `provider.rs`（731 行 → 3 文件）

| 目标文件 | 内容 | 行数预算 |
|----------|------|----------|
| `provider/mod.rs` | mod 声明 + 重导出 | ~20 行 |
| `provider/db_value.rs` | `DbValue` enum + `Display` + `From` impls + `DbValueConvertError` + `TryFrom` impls + `From<DbValueConvertError> for EFError` | ~400 行 |
| `provider/traits.rs` | `ISqlGenerator` + `IsolationLevel` + `IAsyncConnection` + `IDatabaseProvider` | ~200 行 |

#### 3.3 拆分 `change_executor.rs`（728 行 → 3 文件）

| 目标文件 | 内容 | 行数预算 |
|----------|------|----------|
| `change_executor/mod.rs` | mod 声明 + `pub use executor::ChangeExecutor` | ~15 行 |
| `change_executor/executor.rs` | `ChangeExecutor` struct + 4 `execute_*` 方法 | ~350 行 |
| `change_executor/sql_gen.rs` | `build_where_with_concurrency` + `generate_insert/update/delete_sql` + `collect_*_params` | ~200 行 |

#### 3.4 拆分 `query/ast.rs`（518 行 → 略超 18 行）

方案：将大型 enum 的 `Display`/`Debug` impl 或辅助函数提取到 `query/ast_impl.rs`（~100 行），主文件降到 ~420 行。

**迭代 3 验证检查点**：
- [ ] `query/builder.rs` → `query/builder/`（6 文件）
- [ ] `provider.rs` → `provider/`（3 文件）
- [ ] `change_executor.rs` → `change_executor/`（3 文件）
- [ ] `query/ast.rs` ≤500 行
- [ ] core crate 内无 .rs 文件 >500 行
- [ ] `cargo check + clippy + test + fmt` 全通过

---

### 迭代 4：macros 拆分（P1）

#### 4.1 拆分 `crates/macros/src/entity.rs`（979 行）

执行前需 Read 全文确认符号边界。预期拆分（`EntityType` derive 宏）：

| 目标文件 | 内容（预期） | 行数预算 |
|----------|-------------|----------|
| `entity/mod.rs` | mod 声明 + 重导出 | ~15 行 |
| `entity/parse.rs` | 解析 `#[derive(EntityType)]` 输入：字段、属性、导航 | ~300 行 |
| `entity/meta_gen.rs` | 生成 `IEntityType::meta()` 实现 | ~250 行 |
| `entity/snapshot_gen.rs` | 生成 `IEntitySnapshot` 实现 | ~200 行 |
| `entity/key_gen.rs` | 生成 `IGetKeyValues` 实现 | ~150 行 |

#### 4.2 拆分 `crates/macros/src/linq/parse.rs`（963 行）

| 目标文件 | 内容（预期） | 行数预算 |
|----------|-------------|----------|
| `linq/parse/mod.rs` | mod 声明 + 重导出 | ~15 行 |
| `linq/parse/expr.rs` | 表达式 AST 解析 | ~300 行 |
| `linq/parse/clause.rs` | from/where/select/join/orderby 等子句解析 | ~350 行 |
| `linq/parse/stream.rs` | token 流处理 | ~250 行 |

#### 4.3 拆分 `crates/macros/src/linq/compile.rs`（798 行）

| 目标文件 | 内容（预期） | 行数预算 |
|----------|-------------|----------|
| `linq/compile/mod.rs` | mod 声明 + 重导出 | ~15 行 |
| `linq/compile/query.rs` | 查询体编译 | ~300 行 |
| `linq/compile/terminal.rs` | 终端方法编译（to_list/first/count 等） | ~250 行 |
| `linq/compile/helpers.rs` | 辅助函数 | ~200 行 |

**迭代 4 验证检查点**：
- [ ] macros crate 无 .rs 文件 >500 行
- [ ] `cargo check --workspace --all-features` + `cargo test` 通过
- [ ] clippy + fmt 通过

---

### 迭代 5：测试文件拆分（P1）

#### 5.1 拆分 `cascade_save_tests.rs`（689 行）

| 目标文件 | 测试场景 |
|----------|----------|
| `tests/cascade_save_pk_backfill.rs` | 一对多 PK 回填 |
| `tests/cascade_save_self_referential.rs` | 自引用更新 |
| `tests/cascade_save_m2m.rs` | 多对多关联 |
| `tests/cascade_save_ordering.rs` | 更新/删除顺序 |

公共辅助提取到 `tests/common/cascade_fixtures.rs`。

#### 5.2 拆分 `navigation_perf_tests.rs`（687 行）

| 目标文件 | 测试维度 |
|----------|----------|
| `tests/nav_perf_include.rs` | Include 深度性能 |
| `tests/nav_perf_lazy.rs` | 延迟加载性能 |
| `tests/nav_perf_batch.rs` | 批量导航性能 |

#### 5.3 拆分 `having_pagination_dialect_tests.rs`（554 行）

将 HAVING 和分页测试拆为两个文件，或内联优化降到 500 以内（执行时根据实际内容选择）。

**迭代 5 验证检查点**：
- [x] tests 目录无 .rs 文件 >500 行
- [x] `cargo test --workspace --all-features -- --skip postgres --skip mysql` 全通过
- [x] 测试数量不减少（12+3+20=35 测试全部保留）

---

### 迭代 6：代码质量提升（P2）

#### 6.1 panic 路径收敛

**范围**：仅 `src/` 生产代码，不含 `tests/` 和 `benches/`。

**重点文件**（当前 panic 数）：
- `db_context/save_pipeline.rs`：30 → 用 `?` / `ok_or_else` 替换
- `di.rs`：10
- `db_context/save_phases.rs`：8
- `db_context/set_ops.rs`：4
- `query/builder/`（拆分后）：4
- `metadata_cache.rs`：3
- `transaction.rs`：3
- `db_context/context.rs`（拆分后）：2
- `navigation_loader.rs`：1
- `query/state.rs`：1

**策略**：
1. `.unwrap()` → `?` 或 `.ok_or_else(|| EFError::X(...))?`
2. `.expect("msg")` → `ok_or_else` 带上下文
3. `panic!()` → `return Err(EFError::X(...))`
4. `unreachable!()` → `EFError::configuration` 防御性返回
5. **例外**：`Mutex::lock().unwrap_or_else(|p| p.into_inner())` 是有意的中毒恢复模式，保留

**验证**：生产代码 panic ≤20（仅保留 Mutex 中毒恢复和少数 `#[cfg(test)]` 路径）。

#### 6.2 EFErrorCode 错误码体系

为 `EFError` 增加可程序化区分的错误码，**不修改枚举变体结构**（向后兼容）：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EFErrorCode {
    ConnectionRefused, ConnectionTimeout, QuerySyntax, QueryTimeout,
    EntityNotFound, ModelValidation, MigrationConflict, ProviderUnsupported,
    ConfigurationInvalid, ChangeTrackingCorrupted, TransactionAborted,
    TransactionDeadlock, ConcurrencyConflict, TypeConversionFailed, Unknown,
}

impl EFError {
    pub fn code(&self) -> EFErrorCode { /* 按变体 + 消息模式匹配 */ }
}
```

#### 6.3 MetadataCache Mutex → RwLock

```rust
// Before: by_key: Mutex<HashMap<Option<String>, Arc<BuiltMetadata>>>
// After:  by_key: RwLock<HashMap<Option<String>, Arc<BuiltMetadata>>>
```

- `get_or_build` 读路径用 `read()`，miss 时升级 `write()`
- 中毒恢复：`RwLock::into_inner()` 同样可用
- 保留 `test_poison_recovery` 测试

#### 6.4 dead_code 清理（9 处）

逐一评估：
- `crates/macros/src/linq/ast.rs`（1 处）：确认是否为宏展开残留
- `crates/core/tests/fk_on_delete_tests.rs`（8 处）：确认测试辅助函数是否未使用

真正死代码删除；误报的改用 `#[allow(unused, reason = "...")]` 带理由。

#### 6.5 最终 mod.rs 全量审查

扫描所有 `mod.rs`，确认仅含 `mod` 声明和 `pub use` 重导出。含业务代码的重构到子文件。

**迭代 6 验证检查点**：
- [ ] 生产代码 panic ≤20
- [ ] `EFErrorCode` 可通过 `error.code()` 获取
- [ ] `MetadataCache` 使用 `RwLock`
- [ ] 所有 `#[allow(dead_code)]` 有明确理由或已删除
- [ ] 所有 `mod.rs` 仅含声明和重导出
- [ ] 全量测试通过

---

### 迭代 7：文档同步 + 版本发布（P2）

#### 7.1 文档更新

- **CHANGELOG.md**：新增 v1.6.0 条目，汇总迭代 2-6 所有变更
- **README.md**：更新架构说明（如有必要）
- **CLAUDE.md**：无需更新（规范不变）

#### 7.2 版本发布

1. 工作区版本 `1.5.3` → `1.6.0`（内部重构 + `EFErrorCode`，minor bump）
2. 各 crate Cargo.toml 版本同步
3. 验证：`cargo check + clippy + test + fmt`
4. Dry-run：`cargo publish --dry-run -p rust-ef-macros`
5. 按依赖顺序发布：macros → core → postgres/mysql/sqlite → cli
6. git commit + tag `v1.6.0` + push

**迭代 7 验证检查点**：
- [ ] CHANGELOG.md 含 v1.6.0 条目
- [ ] 所有 crate 版本一致（1.6.0）
- [ ] `cargo publish --dry-run` 全部成功
- [ ] git tag v1.6.0 已推送

---

## 三、全局验证（每个迭代结束前执行）

```powershell
# 1. 编译检查（所有 feature）
cargo check --workspace --all-features

# 2. Clippy（所有 feature + 所有 target，warning 即错误）
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3. 格式化检查
cargo fmt --all --check

# 4. SQLite 测试（无需外部数据库）
cargo test --workspace --all-features -- --skip postgres --skip mysql

# 5. 文件行数检查（确认无 >500 行文件）
Get-ChildItem -Path crates -Recurse -Filter *.rs | ForEach-Object {
    $lines = (Get-Content $_.FullName | Measure-Object -Line).Lines
    if ($lines -gt 500) { Write-Host "$($_.FullName): $lines lines" }
}

# 6. mod.rs 合规检查（确认仅含 mod 声明 + pub use）
Get-ChildItem -Path crates -Recurse -Filter "mod.rs" | ForEach-Object {
    $lines = (Get-Content $_.FullName | Measure-Object -Line).Lines
    if ($lines -gt 50) { Write-Host "SUSPECT: $($_.FullName): $lines lines" }
}
```

---

## 四、假设与决策

### 假设
1. `migration/` 目录下 10 个子文件内容已正确（基于 v1 计划执行记录），仅需删除原文件即可恢复编译。
2. 跨文件 `impl Struct` 在同一 crate 内合法，结构体字段需 `pub(crate)` 或 `pub(super)`。
3. `query/mod.rs` 中 `mod builder;` 在 `builder.rs` → `builder/` 转换后无需修改（Rust 自动解析）。
4. 外部 API（`prelude` 导出、公开 trait 方法签名）在所有迭代中保持不变。

### 决策
1. **迭代顺序**：P0 阻塞修复 → P0 mod.rs 合规 → P0 大文件 → P1 macros → P1 测试 → P2 质量 → P2 发布。
2. **mod.rs 合规优先级提升**：v1 计划未充分覆盖此问题，v2 将其纳入迭代 2（与阻塞修复同级）。
3. **panic 收敛不修改 EFError 枚举结构**：保持向后兼容，通过 `code()` 方法扩展。
4. **不新增功能**：所有迭代仅清理重构 + 质量提升。

### 风险
1. **跨文件 impl 可见性**：拆分后子文件需访问结构体私有字段，需 `pub(crate)` 或 `pub(super)`。
2. **测试拆分可能导致 fixture 丢失**：需将公共 entity 定义提取到 `tests/common/`。
3. **macros 拆分风险较高**：proc-macro 涉及 `quote!`/`parse_macro_input!`，拆分时需确保 token 流传递正确。迭代 4 执行前需完整阅读源文件。

---

## 五、执行顺序总览

| 迭代 | 优先级 | 内容 | 预计文件操作 |
|------|--------|------|-------------|
| 2 | P0 | 删除 migration.rs + db_context/mod.rs 合规 + tests/common/mod.rs 合规 | -1, +3 文件 |
| 3 | P0 | query/builder + provider + change_executor + ast 拆分 | +14, -3 文件 |
| 4 | P1 | macros 拆分（entity + linq/parse + linq/compile） | +12, -3 文件 |
| 5 | P1 | 测试文件拆分 | +8, -3 文件 |
| 6 | P2 | panic 收敛 + EFErrorCode + RwLock + dead_code + mod.rs 审查 | 修改 ~20 文件 |
| 7 | P2 | 文档 + 发布 v1.6.0 | 修改 ~7 文件 |
