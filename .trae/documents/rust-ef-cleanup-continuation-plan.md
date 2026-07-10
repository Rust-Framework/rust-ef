# rust-ef 清理迭代计划（续篇：迭代 2-6）

## 背景

本计划是 [rust-ef-cleanup-iteration-plan.md](./rust-ef-cleanup-iteration-plan.md) 的续篇。
迭代 1 已完成并发布为 v1.5.3（commit `9e442c8`），包含：

- MySQL tracing 编译失败修复
- migration/ 死代码目录清理（1,298 行）
- CI 加固（`--all-features` clippy + test）
- 4 个 clippy `--all-features` 错误修复
- 代码格式化统一

本计划覆盖剩余的迭代 2-6，目标仍是**仅清理与修复，不包含新功能开发**。

---

## 当前状态分析（迭代 1 后实测）

### 超 500 行的源文件（按行数降序）

| 文件 | 实测行数 | 倍数 | 主要职责 |
|------|---------|------|----------|
| `crates/core/src/db_context.rs` | 1,730 | 3.5x | Options/Builder + ErasedSetOps + SetOps + DbContext + SaveChanges 管线 |
| `crates/core/src/migration.rs` | 1,687 | 3.4x | 快照类型 + 方言 + Diff + SQL 生成 + 历史 + JSON I/O |
| `crates/core/src/query/builder.rs` | 1,219 | 2.4x | QueryBuilder 71 个方法（filter/join/CTE/聚合/终端） |
| `crates/macros/src/entity.rs` | 979 | 2.0x | EntityType 宏：属性解析 + 代码生成 |
| `crates/macros/src/linq/parse.rs` | 963 | 1.9x | linq! DSL 解析（40 个函数） |
| `crates/macros/src/linq/compile.rs` | 798 | 1.6x | linq! 编译（24 个函数） |
| `crates/core/src/provider.rs` | 731 | 1.5x | DbValue + TryFrom + ISqlGenerator/IDatabaseProvider trait |
| `crates/core/src/change_executor.rs` | 728 | 1.5x | ChangeExecutor：INSERT/UPDATE/DELETE/UPSERT 执行 |
| `crates/core/src/query/ast.rs` | 518 | 1.0x | 查询 AST 类型（边界，优先级低） |

### 超 500 行的测试文件

| 文件 | 行数 | 处理策略 |
|------|------|---------|
| `crates/core/tests/cascade_save_tests.rs` | 689 | 迭代 3 拆分 |
| `crates/core/tests/navigation_perf_tests.rs` | 687 | 迭代 3 拆分 |
| `crates/core/tests/having_pagination_dialect_tests.rs` | 554 | 迭代 3 拆分 |

### 代码健康问题

| # | 问题 | 实测数据 |
|---|------|---------|
| 1 | 运行时 panic 路径 | 877 处 `unwrap/expect/panic!` 跨 56 文件（含测试）；src 下重点：db_context.rs 44 处、di.rs 10 处、query/builder.rs 4 处、metadata_cache.rs 3 处、transaction.rs 3 处 |
| 2 | EFError 无错误码 | 12 变体全为 `(String, Option<BoxError>)` |
| 3 | MetadataCache 用 Mutex | 读多写少场景应 RwLock |
| 4 | TODO/FIXME/HACK | 仅 2 处（bool_expr_tests 的 `unimplemented!()` 占位 + compile.rs 注释引用），无需清理 |

### 模块化参考

`crates/core/src/query/` 和 `crates/core/src/observability/` 已是规范模块目录，`mod.rs` 仅含声明和 re-export，可作为拆分模板。

---

## 迭代 2：核心模块拆分（db_context.rs + migration.rs）— P0

**目标**：将两个最大文件拆为模块目录，满足单一职责与 500 行限制。

### 2.1 拆分 db_context.rs（1,730 行 → `db_context/` 目录）

**当前结构**（实测符号位置）：
- L85-335：`DbContextOptions` + `DbContextOptionsBuilder` + `Debug` impl + `redact_connection_string`
- L336-436：`ErasedSetOps` trait（100 行）
- L437-777：`SetOps<E>` struct + impl（340 行）
- L778-807：`resolve_delete_behavior`（pub(crate)）
- L808-1008：`DbContext` struct + 第一个 impl 块（from_options/set/model/ensure_created 等）
- L1009-1809：`DbContext` 第二个 impl 块（save_changes/use_transaction 等，800 行）
- L1810-1830：`SaveChangesResult` + Display impl

**拆分目标**：

| 新文件 | 来源 | 内容 | 预估行数 |
|--------|------|------|----------|
| `db_context/mod.rs` | — | `mod` 声明 + `pub use` re-export | ~30 |
| `db_context/options.rs` | L85-335 | `DbContextOptions` + Builder + Debug + `redact_connection_string` | ~260 |
| `db_context/set_ops.rs` | L336-777 | `ErasedSetOps` trait + `SetOps<E>` + impl | ~445 |
| `db_context/save_pipeline.rs` | L1009-1809 内的 save 编排 + L1810-1830 | `save_changes` 方法提取为自由函数 + `SaveChangesResult` | ~350 |
| `db_context/mod.rs`（DbContext 主体） | L778-1008 + 其余 impl | `resolve_delete_behavior` + `DbContext` struct + 非 save 的 impl | ~350 |

**关键约束**：
- `mod.rs` 仅 `mod` 声明 + `pub use`，无业务代码（project_memory 硬约束）
- 跨文件 impl 需将 `SetOps<E>` 字段、`DbContext` 字段设为 `pub(crate)`
- `save_changes` 方法体内联调用提取到 `save_pipeline.rs` 的自由函数，方法本身保留在 mod.rs 作薄封装
- `lib.rs:28` 的 `pub mod db_context;` 不变（Rust 自动识别目录）
- prelude 的 re-export 路径不变

**验证**：
- `cargo check --workspace --all-features` 通过
- `cargo test --workspace` 全部通过
- `db_context/mod.rs` ≤ 500 行且仅含声明和 re-export

### 2.2 拆分 migration.rs（1,687 行 → `migration/` 目录）

**当前结构**（实测符号位置）：
- L15-67：快照类型（`Migration`/`ModelSnapshot`/`SnapshotEntityType`/`SnapshotColumn`）
- L68-188：`MigrationDialect` enum + impl（`quote()`/`map_column_type()`）
- L189-236：`SchemaChange` enum
- L237-473：`MigrationEngine` struct + 第一个 impl（`new`/`generate`/`diff`）
- L474-682：diff 辅助函数（`fk_target`/`fk_reference_for_property`/`resolve_fk_on_delete_clause`/`diff_foreign_keys`/`diff_indexes` 等）
- L683-1374：`MigrationEngine` 第二个 impl（SQL 生成，691 行）
- L1375-1540：历史追踪（`MigrationHistoryEntry`/`MigrationStore`/`seed_insert_sql`/`split_sql_statements`/`create_migration_history_table_sql`）
- L1541-1687：JSON I/O（`parse_model_snapshot_json`/`snapshot_to_json`/`snapshot_from_json`/`extract_json_string`/`extract_quoted_after_colon`）

**拆分目标**：

| 新文件 | 来源 | 内容 | 预估行数 |
|--------|------|------|----------|
| `migration/mod.rs` | — | `mod` 声明 + `pub use` re-export | ~40 |
| `migration/types.rs` | L15-67 | 4 个快照 struct | ~55 |
| `migration/dialect.rs` | L68-188 | `MigrationDialect` enum + impl | ~120 |
| `migration/diff.rs` | L189-236 + L474-682 | `SchemaChange` enum + diff 辅助函数 | ~470 |
| `migration/engine.rs` | L237-473 | `MigrationEngine` struct + `new`/`generate`/`diff` impl | ~240 |
| `migration/engine_sql_create.rs` | L683-~1000（CREATE/DROP TABLE 部分） | `generate_up_sql`/`generate_down_sql`/`create_table_sql`/`drop_table_sql` | ~320 |
| `migration/engine_sql_alter.rs` | L~1000-1374（ALTER/INDEX/FK 部分） | `alter_column_sql`/`add_foreign_key_sql`/`drop_foreign_key_sql`/`create_index_sql`/`drop_index_sql` | ~375 |
| `migration/history.rs` | L1375-1540 | `MigrationHistoryEntry`/`MigrationStore` + 历史 SQL | ~165 |
| `migration/snapshot_io.rs` | L1541-1687 | JSON 序列化/反序列化 + 辅助函数 | ~145 |

**关键约束**：
- 多个 `impl MigrationEngine` 块分布在 `engine.rs`/`engine_sql_create.rs`/`engine_sql_alter.rs`（Rust 允许）
- `mod.rs` 仅声明 + re-export
- **必须保留 fk_on_delete 支持**（迭代 1 删除的死代码目录正是缺失此功能）
- `lib.rs:38` 的 `pub mod migration;` 不变
- 所有 `pub` 可见性不变，公共 API 路径不变

**验证**：
- `cargo check --workspace --all-features` 通过
- `cargo test --workspace` 全部通过（含 `fk_on_delete_tests` 6 个测试）
- `migration/mod.rs` 仅含声明和 re-export
- 所有子文件 ≤ 500 行

---

## 迭代 3：次要模块拆分 — P1

**目标**：拆分其余超 500 行的非宏源文件 + 超限测试文件。

### 3.1 拆分 query/builder.rs（1,219 行）

**当前结构**：单文件 71 个方法，方法分组明确：
- L52-262：构造 + filter 系列（filter/filter_column/filter_in/filter_not/filter_is_null/filter_like/or_where/where_exists/where_in_subquery）
- L195-220：order_by + distinct
- L328-410：find/exists + skip/take
- L421-585：include/then_include + join 系列（inner/left/right/full/cross）
- L586-840：group_by/having/window/CTE/set ops（union/intersect/except）
- L850-950：聚合（sum/avg/min/max）
- L954-990：select_internal + SQL 编译（to_sql/compile_sql）
- L1004-1273：终端方法（to_list/first/count/any/last/single/long_count/all/contains/to_dictionary/execute_update/execute_delete）

**拆分策略**（`query/` 目录已存在，新增子模块，`builder.rs` 保留 struct + 构造 + 核心）：

| 新文件 | 提取方法 | 预估行数 |
|--------|---------|----------|
| `query/builder.rs`（保留） | struct + new + with_provider + state + filter + order_by + skip/take | ~350 |
| `query/builder_filter.rs` | filter_column/filter_not/filter_in/filter_not_in/filter_is_null/filter_is_not_null/filter_between/filter_like/filter_not_like/or_where/where_exists_internal/where_in_subquery_internal + apply_query_filter | ~250 |
| `query/builder_join.rs` | include_internal/then_include_internal/inner_join_internal/left_join_internal/right_join_internal/full_join_internal/cross_join_internal | ~200 |
| `query/builder_set.rs` | group_by_internal/having_internal/having_expr_internal/window_internal/with_cte_internal/with_cte_typed/with_recursive_cte_typed/from_cte/union_internal/union_all_internal/intersect_internal/except_internal | ~280 |
| `query/builder_terminal.rs` | find/find_by_key/exists_by_id/exists_by_key/to_list/to_list_with_includes/first/first_or_default/count/any/last/last_or_default/single/single_or_default/long_count/all/contains/to_dictionary/execute_update/execute_delete + 聚合 sum/avg/min/max + select_internal/to_sql/compile_sql | ~450 |

**约束**：`query/mod.rs` 增加新 `mod` 声明，`pub use` 保持不变。

### 3.2 拆分 provider.rs（731 行）

**当前结构**：
- L20-194：`DbValue` enum + Display + From impls（含 L75-78 hex mod）
- L196-214：`DbValueConvertError`
- L215-587：`TryFrom<DbValue>` impls（15 个目标类型，372 行）
- L588-728：`ISqlGenerator` trait + `IsolationLevel` + `IAsyncConnection` + `IDatabaseProvider`

**拆分策略**（`provider.rs` → `provider/` 目录）：

| 新文件 | 内容 | 预估行数 |
|--------|------|----------|
| `provider/mod.rs` | `mod` 声明 + `pub use` re-export | ~25 |
| `provider/db_value.rs` | `DbValue` + Display + From + `DbValueConvertError` + hex mod | ~250 |
| `provider/conversions.rs` | 15 个 `TryFrom<DbValue>` impl | ~375 |
| `provider/traits.rs` | `ISqlGenerator` + `IsolationLevel` + `IAsyncConnection` + `IDatabaseProvider` | ~140 |

### 3.3 拆分 change_executor.rs（728 行）

**当前结构**：
- L15-16：`ChangeExecutor` struct（ZST）
- L17-644：impl ChangeExecutor（6 个方法：execute_inserts/execute_upserts/execute_updates/execute_updates_per_row/execute_deletes/execute_deletes_per_row）
- L645-689：`build_where_with_concurrency`
- L691-728：SQL 生成自由函数（generate_insert_sql/generate_update_sql/generate_delete_sql/collect_insert_params/collect_update_params/collect_delete_params）

**拆分策略**（`change_executor.rs` → `change_executor/` 目录）：

| 新文件 | 内容 | 预估行数 |
|--------|------|----------|
| `change_executor/mod.rs` | `ChangeExecutor` struct + `mod` 声明 + `pub use` | ~30 |
| `change_executor/insert.rs` | `execute_inserts` + `execute_upserts` + `generate_insert_sql` + `collect_insert_params` | ~280 |
| `change_executor/update.rs` | `execute_updates` + `execute_updates_per_row` + `generate_update_sql` + `collect_update_params` + `build_where_with_concurrency` | ~270 |
| `change_executor/delete.rs` | `execute_deletes` + `execute_deletes_per_row` + `generate_delete_sql` + `collect_delete_params` | ~180 |

### 3.4 拆分超限测试文件

| 测试文件 | 行数 | 拆分策略 |
|---------|------|---------|
| `cascade_save_tests.rs`（689） | 按测试场景拆为 `tests/cascade_save/` 目录：`mod.rs` + `one_to_many.rs` + `self_referential.rs` + `m2m.rs` + `ordering.rs` + `set_null.rs` + `unloaded.rs` |
| `navigation_perf_tests.rs`（687） | 按性能场景拆为 `tests/navigation_perf/` 目录 |
| `having_pagination_dialect_tests.rs`（554） | 按 having/pagination/dialect 拆为 3 个子模块 |

**约束**：测试拆分需保持 `cargo test` 运行行为不变，每个子模块用 `#[mod_test]` 风格或独立测试目标。

---

## 迭代 4：代码健康（panic 收敛 + 错误码 + 锁优化）— P1

### 4.1 收敛运行时 panic 路径

**分类策略**：
- **保留 panic**：内部不变量违反（如元数据在 `ensure_created` 后必存在）、`unreachable!()` 逻辑不可达分支——添加 `// SAFETY: invariant` 注释
- **转为 Result**：用户可控入口（`ctx.set::<T>()` 未注册实体、配置错误、类型转换失败、连接获取失败）

**重点文件**（src 下 panic 密度）：
1. `db_context.rs`（拆分后为 `db_context/` 子模块）— 44 处
2. `di.rs` — 10 处
3. `query/builder.rs`（拆分后）— 4 处
4. `metadata_cache.rs` — 3 处
5. `transaction.rs` — 3 处

**改动模式**：
- `EntityNotRegistered` 类 panic → `EFError::Configuration("entity type T not registered")`
- `expect("DbContext")` 构造 panic → `EFError`
- 内部 `unwrap()` 保留但加注释说明不变量
- 测试代码的 `unwrap()` 不动（测试惯用）

**验证**：`grep -r "\.unwrap()\|\.expect(\|panic!" crates/*/src/ | wc -l` 显著下降（目标：src 下 <30 处）

### 4.2 EFError 错误码体系

**文件**：`crates/core/src/error.rs`

**改动**：
- 新增 `EFErrorCode` 枚举：`ConnectionFailed`/`QueryExecution`/`EntityNotFound`/`ModelValidation`/`MigrationError`/`ProviderError`/`ConfigError`/`ChangeTrackingError`/`TransactionError`/`ConcurrencyConflict`/`TypeConversionError`/`Other`
- 为 `EFError` 增加 `code(&self) -> EFErrorCode` 方法
- 可选：`is_transient(&self) -> bool` 标记可重试错误（Connection、Transaction）

**约束**：不破坏现有 `EFError` 变体结构和 `Display` 实现；`EFErrorCode` 为 `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`

### 4.3 MetadataCache Mutex → RwLock

**文件**：`crates/core/src/metadata_cache.rs`

**改动**：`Mutex<MetadataCache>` → `RwLock<MetadataCache>`，读路径 `.read()`，写路径 `.write()`

**约束**：保留 poisoned lock 恢复逻辑（project_memory 硬约束）

---

## 迭代 5：宏代码拆分 — P2

### 5.1 拆分 macros/entity.rs（979 行）

**当前结构**：
- L10：`expand_entity_type` 入口函数
- L732-989：导航/属性辅助函数（`NavigationDiscriminant`/`NavTypeInfo`/`detect_navigation_type`/`extract_*`/`generate_*`）

**拆分策略**（`entity.rs` → `entity/` 目录）：

| 新文件 | 内容 | 预估行数 |
|--------|------|----------|
| `entity/mod.rs` | `expand_entity_type` 入口 + `mod` 声明 + re-export | ~350 |
| `entity/attrs.rs` | `extract_*` 属性解析函数（foreign_key_target/through_type/foreign_key_field_name/table_name/context_key/column_name/sequence_name/on_delete/max_length） | ~200 |
| `entity/navigation.rs` | `NavigationDiscriminant`/`NavTypeInfo`/`detect_navigation_type`/`is_navigation_field`/`type_ident_string`/`is_unit_type` | ~180 |
| `entity/codegen.rs` | `generate_parse_expr`/`generate_scalar_parse` + 字段代码生成 | ~250 |

**约束**：`#[proc_macro_derive(EntityType)]` 入口必须留在 crate root 可见位置（lib.rs 的 `#[proc_macro_derive]` 声明不变），内部逻辑提取到子模块。

### 5.2 拆分 macros/linq/parse.rs（963 行）

**当前结构**：40 个函数，按 DSL 语法形式分组：
- L21-50：`LinqInput` Parse impl
- L51-119：值/字段/索引解析
- L120-228：`parse_query` 主入口
- L228-398：source/closure 解析
- L399-518：clause 分发（`LinqClause` Parse + `parse_*_rest`）
- L519-760：window 解析
- L761-970：having/join/set 解析

**拆分策略**（`linq/parse.rs` → `linq/parse/` 目录）：

| 新文件 | 内容 | 预估行数 |
|--------|------|----------|
| `linq/parse/mod.rs` | `LinqInput`/`LinqClause` Parse impl + `parse_query` + `mod` 声明 | ~350 |
| `linq/parse/value.rs` | 值/字段/索引解析（`ValueKind`/`parse_value_filter`/`parse_value_index_or_key`/`parse_field_or_tuple`） | ~120 |
| `linq/parse/clause.rs` | clause 分发（`parse_include_rest`/`parse_order_by_rest`/`parse_group_by_rest`/`parse_select_rest`/`parse_with_rest`/`parse_from_rest`/`parse_set_rest`/`parse_join_rest`） | ~300 |
| `linq/parse/window_having.rs` | window + having 解析（`parse_window_rest`/`parse_window_field_list`/`parse_window_order_list`/`parse_having_rest`/`expr_to_having_ast`/`parse_having_compare_from_binary`/`parse_agg_call`） | ~280 |

### 5.3 拆分 macros/linq/compile.rs（798 行）

**当前结构**：24 个函数，按编译阶段分组：
- L30-119：bool 表达式编译
- L120-310：comparison/method 编译
- L312-420：negation/contains 编译
- L423-670：subquery 编译
- L674-798：method/order/having 编译

**拆分策略**（`linq/compile.rs` → `linq/compile/` 目录）：

| 新文件 | 内容 | 预估行数 |
|--------|------|----------|
| `linq/compile/mod.rs` | `compile_bool_expr`/`compile_expr` 入口 + `mod` 声明 + re-export | ~280 |
| `linq/compile/bool.rs` | `compile_bool_comparison`/`compile_bool_method`/`compile_not`/`compile_bool_member`/`compile_comparison`/`compile_negated_comparison`/`compile_contains` | ~280 |
| `linq/compile/subquery.rs` | `SubqueryKind`/`extract_subquery_closure`/`compile_subquery_*`/`compile_not_subquery`/`compile_in_subquery_*` | ~280 |

---

## 迭代 6：文档同步 + 最终发布 — P2

### 6.1 修正 PRODUCTION_READINESS_SPEC.md

**文件**：`docs/PRODUCTION_READINESS_SPEC.md`

**修正内容**：
- 测试数量：声称 278 → 实际数量（重新统计 `cargo test --workspace` 结果）
- db_context.rs 行数：声称 ~700 → 拆分后的模块结构
- 各维度就绪裁定：根据迭代 2-5 实际结果更新
- 新增"已知限制"章节：Identity Resolution 未实现、连接重试未实现、SemVer v1.5.2 违规记录

### 6.2 更新 CHANGELOG.md

新增 v1.5.4 条目，记录全部清理改动：
- Changed：模块拆分（db_context/migration/query/provider/change_executor/macros）
- Fixed：panic 收敛、EFError 错误码、MetadataCache RwLock
- Removed：死代码（迭代 1 已记，此处仅补充迭代 2-6 移除的冗余）

### 6.3 最终验证

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- 所有 src 文件 ≤ 500 行（宏入口 mod.rs 除外）
- 所有 mod.rs 仅含 `mod` 声明和 `pub use`
- `grep -r "TODO\|FIXME\|HACK\|XXX" crates/*/src/` 无结果
- `grep -r "unwrap()\|expect(\|panic!" crates/*/src/ | wc -l` < 30

### 6.4 发布

- 版本号：1.5.3 → 1.5.4（patch，纯清理无 API 破坏）
- 若迭代 4 的 panic→Result 导致公共 API 签名变化，升 minor 为 1.6.0
- 发布到 crates.io（macros → core → postgres → mysql → sqlite）

---

## 假设与决策

1. **范围限定**：本计划仅清理冗余/无效/违规内容，不包含新功能（Identity Resolution、连接重试、继承映射等另行规划）
2. **API 兼容**：所有拆分保持公共 API 路径不变（`pub use` re-export 维持原路径），不破坏下游用户代码
3. **mod.rs 纯净性**：所有 mod.rs 仅含 `mod` 声明和 `pub use` re-export，禁止业务代码（project_memory 硬约束）
4. **跨文件 impl**：Rust 允许 `impl Struct` 分布在多个文件，需将 struct 字段设为 `pub(crate)`
5. **panic 处理边界**：保留内部不变量 panic（带注释），仅转换用户可控入口的 panic 为 Result
6. **宏入口保留**：`#[proc_macro_derive]`/`#[proc_macro]` 入口函数留在 crate root 可见位置，内部逻辑提取到子模块
7. **测试文件拆分**：超 500 行的测试文件同样拆分（project_memory 硬约束适用于所有 .rs 文件）
8. **版本策略**：纯拆分发 patch（1.5.4）；若 panic→Result 改变公共签名则升 minor（1.6.0）

## 验证检查点

每轮迭代结束均需通过：
- [ ] `cargo check --workspace --all-features` 无错误
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` 无警告
- [ ] `cargo test --workspace` 全部测试通过
- [ ] `cargo fmt --all --check` 通过
- [ ] 所有源文件 ≤ 500 行（宏入口 mod.rs 除外）
- [ ] 所有 mod.rs 仅含 `mod` 声明和 `pub use`
- [ ] 无 `TODO`/`FIXME`/`HACK`/`XXX` 注释（src 下）
- [ ] 无 `unsafe` 代码

## 执行顺序

迭代 2 → 迭代 3 → 迭代 4 → 迭代 5 → 迭代 6

每轮迭代完成后提交一次 commit，迭代 6 完成后统一发布。若迭代中发现阻断性问题，立即停止并修复后再继续。
