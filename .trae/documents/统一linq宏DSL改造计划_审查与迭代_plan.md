# 统一 `linq!` 宏 DSL 改造计划 — 审查与迭代

> 版本: 对 `统一linq宏DSL改造计划_plan.md`（v0.4 Beta 1）的完整性审查与遗漏补全
> 审查日期: 2026-06-26
> 审查方法: 逐项比对计划主张与 `d:\GitCode\RF\rust-ef` 实际代码状态

---

## 一、审查结论摘要

**核心结论**：原计划描述的「当前状态」与「待办事项」**约 90% 已在代码库中实现**，但计划文档从未同步更新，仍以「待改造」口吻描述已完成的工作。这会导致后续执行者重复劳动、按失效的行号/函数名/签名去改不存在的代码。

| 类别 | 数量 | 处置 |
|---|---|---|
| 已完成（计划当作 to-do，实际已落地） | 9 项 | 仅在计划中标注「已完成」，不重复实现 |
| 真实遗漏（需迭代补全） | 4 项 | 本迭代计划主体 |
| 计划文档自身的事实错误（需勘误） | 5 项 | 勘误原计划文档 |

---

## 二、已完成项清单（无需再做，仅供对账）

经逐文件验证，原计划下列主张的「改造目标」**已在代码库中落地**：

1. ✅ **Forms A/B/C 全部实现**（`crates/macros/src/linq.rs`，实际 1592 行，非计划所称 513 行）。`LinqClause` 枚举覆盖 `Include/OrderBy/GroupBy/Select/Having/Sum/Avg/Min/Max/Count/Distinct/Set/InnerJoin/LeftJoin/ExecuteUpdate`，且**额外实现了 `Take`/`Skip`**（原计划未提及）。
2. ✅ **字符串 API 全部移除**：`include_named`/`then_include_named`/`order_by(&str)`/`group_by(&[&str])`/`having(&str)`/`sum(&str)`/`avg(&str)`/`select_columns`/`set_column(&str,_)`/`inner_join(&str,...)`/`left_join(&str,...)` 均已不存在，替换为 `*_internal` 方法（`query.rs` 中 `include_internal`@700、`order_by_column`@581、`sum_internal`@842 等）。
3. ✅ **`find_by_id` bug 已修复**：改为 `find(id: impl Into<DbValue>)`（`query.rs:646`），使用 `T::entity_meta().primary_keys.first()`，不再硬编码 `"id"`。
4. ✅ **`#[foreign_key]` 导航字段 bug 已修复**：`extract_foreign_key_field_name`（`entity.rs:655`）返回 `quote!{ None }`，文档注释（640-654）说明改由 `NavigationMeta` 默认推导。
5. ✅ **`set_property` 死代码已移除**：`entity.rs` 中已无该 accessor 生成代码。
6. ✅ **ModelBuilder 已 DSL 化**：`has_query_filter`（`model_builder.rs:143`）接受 `BoolExpr`；`has_index`@244 / `has_key`@213 接受 `&'static [&'static str]`；`EntityConfig.query_filter: Option<BoolExpr>`（`model_builder.rs:25`，非 `Option<String>`）。
7. ✅ **9 个 LINQ 终端方法全部实现**（`query.rs`）：`last`@1075、`last_or_default`@1087、`single`@1104、`single_or_default`@1121、`long_count`@1137、`all`@1143、`contains`@1154、`to_dictionary`@1164、`distinct`@607。其中 `single`/`single_or_default` 用 `take(2)` 技巧规避了 `QueryBuilder: Clone` 依赖（结构体本身**未派生 `Clone`**，与原计划假设 5.2.1 相反）。
8. ✅ **`LinqCtx` 已扩展为三字段** `{ entity, param, params }`（`linq.rs:1193`），`params: Vec<(Ident, Type)>` 支持 join 多参数闭包。
9. ✅ **`FIELD_*` 导航常量已由 derive 生成**，`field_const(entity, field, kind)`（`linq.rs:1568`，非计划所称 `field_column_const`）以 `FieldKind::{Column, Navigation}` 参数化分叉。

---

## 三、真实遗漏（本迭代主体）

### G1. `min_internal`/`max_internal` 仍返回 `Option<String>`（类型丢失 bug 未修）

**现状**：`crates/core/src/query.rs:894` 与 `:914`，签名均为 `-> EFResult<Option<String>>`，直接返回 `rows.first().and_then(|r| r.first().cloned())`，原类型信息（i32/f64/bool 等）丢失，调用方需手动 parse。

**与原计划偏差**：原计划 §4.3.4 提出改为 `min_internal<V: TryFrom<DbValue>>` 泛型版本，**该改造未落地**。这是原计划验收清单第 6 项「`min`/`max` 返回泛型 `Option<V>`」尚未满足。

**修复方案**：
- `crates/core/src/query.rs`：将 `min_internal`/`max_internal` 改为泛型 `min_internal<V, E>(self, col) -> EFResult<Option<V>> where V: TryFrom<DbValue, Error = E>, E: Into<EFError>`。
- 实现内 `V::try_from(db_value)` 转换，失败时返回 `EFError::Query`。
- 保留 `#[doc(hidden)]` 与现有 `linq!(min b.rating)` 展开对接（宏展开侧无需改，类型由调用点上下文推断）。

### G2. `DbValue` 缺 `TryFrom<DbValue>` 实现（阻塞 G1）

**现状**：全 `crates/core/src/` 搜索 `impl TryFrom<DbValue>` 零命中。原计划假设 5.2.2 已正确识别此缺口，但未实现。

**修复方案**：在 `crates/core/src/query.rs` 或新建 `crates/core/src/db_value.rs`，为常用类型补 `TryFrom<DbValue>`：
- `i32` ← `DbValue::I32(v)` / `I64(v)` 截断 / `Real(v)` 取整
- `i64` ← `I64` / `I32` / `Real`
- `f64` ← `Real` / `I32` / `I64`
- `String` ← `Text` / 其他类型 `to_string()`
- `bool` ← `Bool` / `I64 != 0`
- `Vec<u8>` ← `Blob`

同时为 `EFError` 实现 `From<各 TryFromError>` 或统一用 `E: Into<EFError>` 约束。

### G3. 专用测试文件未创建

**现状**：原计划 §4.6.1 提出新建 `crates/core/tests/linq_dsl_tests.rs` 与 `crates/core/tests/linq_terminal_tests.rs`，实际均不存在（Glob `crates/core/tests/*dsl*` 零命中）。现有 `linq_tests.rs` 覆盖部分但无 DSL 子句形式与终端方法的专项测试。

**修复方案**：
- `crates/core/tests/linq_dsl_tests.rs`：覆盖 Form B 全部子句（include/then_include/order_by/group_by/select/having/sum/avg/min/max/count/distinct/set/inner_join/left_join/execute_update/take/skip）与 Form C（filter/index/key 值产生）。
- `crates/core/tests/linq_terminal_tests.rs`：覆盖 9 个终端方法（last/last_or_default/single/single_or_default/to_dictionary/distinct/all/contains/long_count），含空集、单元素、多元素边界。
- SQLite 内存库为默认后端，复用现有测试 fixtures。

### G4. 文档未同步，且原计划章节映射错误

**现状 1（未同步）**：`docs/rust-ef/` 下相关章节仍描述旧字符串 API。

**现状 2（映射错误）**：原计划 §4.6.3 列出的章节名「04 查询基础 / 05 过滤表达式 / 06 排序与分页 / 07 聚合与分组 / 08 导航加载 / 09 JOIN 查询 / 10 批量操作 / 11 最佳实践」**与实际目录结构完全不符**。实际结构为：

| 计划误称 | 实际目录 |
|---|---|
| 04 查询基础 | `04-relationships/`（关系设计）|
| 05 过滤表达式 | `05-query-patterns/`（查询模式）|
| 06 排序与分页 | `06-advanced-query/`（高级查询）|
| 07 聚合与分组 | `07-change-tracking/`（变更跟踪）|
| 08 导航加载 | `08-bulk-operations/`（批量操作）|
| 09 JOIN 查询 | `09-transactions-migrations/`（事务迁移）|
| 10 批量操作 | `10-di-interceptors/`（DI 拦截器）|
| 11 最佳实践 | `11-best-practices/`（最佳实践，唯一对得上）|

**修复方案**：按实际目录结构同步以下文件（这些是真正需要 `linq!` 化的章节）：
- `docs/rust-ef/05-query-patterns/linq-macro.md` — Form A/B/C 语法总览（最关键）
- `docs/rust-ef/05-query-patterns/filter-sort-page.md` — 过滤 + 排序子句
- `docs/rust-ef/05-query-patterns/dbset-and-queryable.md` — DbSet 与 QueryBuilder
- `docs/rust-ef/05-query-patterns/count-any.md` — count/all/contains 终端
- `docs/rust-ef/06-advanced-query/aggregation.md` — sum/avg/min/max 子句（含 G1 修复后的泛型返回）
- `docs/rust-ef/06-advanced-query/group-by-having.md` — group_by + having 子句
- `docs/rust-ef/06-advanced-query/join-queries.md` — inner_join/left_join 多参数闭包
- `docs/rust-ef/06-advanced-query/global-query-filters.md` — Form C 的 `linq!(filter |b| ...)` 用法
- `docs/rust-ef/04-relationships/eager-loading.md` — `include ... then ...` 子句
- `docs/rust-ef/08-bulk-operations/execute-update.md` — `set` 子句 + `execute_update`
- `docs/rust-ef/11-best-practices/code-review-checklist.md` — 统一 `linq!` 风格清单
- `docs/rust-ef/INDEX.md` + `INDEX.json` — 章节索引同步
- `README.md` — Best Practices 章节示例更新

原计划提到的「新增 `12-linq-terminals/`」目录**不创建**——终端方法参考并入 `05-query-patterns/count-any.md` 与 `06-advanced-query/aggregation.md`，避免目录膨胀。

---

## 四、原计划文档勘误（5 项事实错误）

这些错误会让后续执行者按失效信息操作，需在原 `统一linq宏DSL改造计划_plan.md` 中直接修正（仅改文档，不动代码）：

| # | 位置 | 错误内容 | 修正为 |
|---|---|---|---|
| E1 | §二.1 / §三.1 | 「13 个独立宏」「现有 `linq!` 已具备...」中的 `field_column_const` 函数 | 实际仅 3 个宏入口（`derive_entity_type`/`column`/`linq`），见 `crates/macros/src/lib.rs:9-46`；函数已重命名为 `field_const(entity, field, kind: FieldKind)`，位于 `linq.rs:1568` |
| E2 | §二.1 行号表 | `extract_field@440-476`/`field_column_const@493-499`/`extract_value@501-513`/`compile_expr@221-244`/`compile_order@431-438`/`LinqCtx@216-219` | 实际：`extract_field@1465`、`field_const@1568`、`extract_value@1580`、`compile_expr@1241`、`compile_order@1451`、`LinqCtx@1193`（三字段 `{entity, param, params}`） |
| E3 | §二.4 / §4.1.5 | `extract_foreign_key_field_name@619-624` 返回目标类型名 bug | bug 已修，函数在 `entity.rs:655`，返回 `None`，文档注释 640-654 说明改由 NavigationMeta 推导 |
| E4 | §4.2.3 / §5.2.5 / §三 | `FilterCondition::new(column, "IS NULL", 0)` 带 value 第 3 参 | 实际签名 `FilterCondition::new(column: &str, operator: &str, param_count: usize)`（`query.rs:35`），值通过 `with_values(column, operator, values: Vec<DbValue>)`（`query.rs:51`）传入。计划中所有按值构造的代码示例均不会编译 |
| E5 | §三.3 / §5.1 | 「遵循 project_memory 约定的 split `let` bindings 风格」 | `c:\Users\lusid\.trae-cn\memory\projects\` 下**无 rust-ef 的 project_memory.md**（仅 rust-agent-flow 有）。该风格推荐可保留为「建议」，但不得援引不存在的项目约定作为权威。用户档案仅注明「 dislikes repetitive explanations」，无风格指令 |

---

## 五、实施顺序

```
阶段 0（勘误，独立可做）──► 修正原计划文档 E1-E5
                              │
阶段 1（阻塞 G1 的前置）──────► G2: DbValue TryFrom 实现
                              │
阶段 2（依赖 G2）─────────────► G1: min/max 泛型化
                              │
阶段 3（与 1/2 并行可做）────► G3: 新建两个测试文件，覆盖 G1-G2 的新行为 + 现有 DSL/终端
                              │
阶段 4（最后）───────────────► G4: 按实际目录同步 12 个文档文件 + INDEX + README
```

阶段 0 可独立先行；阶段 1→2 必须顺序；阶段 3、4 可与 1/2 并行。

---

## 六、验证步骤

### 6.1 编译与 lint
```bash
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

### 6.2 测试
```bash
cargo test --workspace
cargo test -p rust-ef --test linq_dsl_tests
cargo test -p rust-ef --test linq_terminal_tests
```
重点验证 G1/G2：`let top: i64 = linq!(set; max b.rating).await?.unwrap_or(0);` 与 `let sum: f64 = linq!(set; sum b.views).await?;` 类型推断通过。

### 6.3 字符串 API 残留再确认
用 Grep 搜索 `\.include_named\(`、`\.order_by("`、`\.sum("`、`find_by_id`、`filter_raw`、`Option<String>` 出现在 `min_internal`/`max_internal` 签名中，期望全零命中。

### 6.4 文档验证
- `docs/rust-ef/` 下被修改的 12 个文件中所有代码块经 `cargo test --doc` 或手动 `rustc --edition 2021 --crate-type lib` 编译验证。
- `INDEX.md` 与 `INDEX.json` 章节标题与文件路径一致。

### 6.5 计划文档勘误验证
重新读取 `统一linq宏DSL改造计划_plan.md`，确认 E1-E5 已修正，不再出现「13 个独立宏」「field_column_const」「FilterCondition::new(col, op, val)」「extract_field@440-476」「project_memory 约定」等表述。

### 6.6 验收清单
- [ ] G1: `min_internal<V>`/`max_internal<V>` 返回 `EFResult<Option<V>>`，泛型 `V: TryFrom<DbValue>`
- [ ] G2: `DbValue` 实现 `TryFrom<DbValue>` for `i32/i64/f64/String/bool/Vec<u8>`
- [ ] G3: `crates/core/tests/linq_dsl_tests.rs` 与 `linq_terminal_tests.rs` 创建并全绿
- [ ] G4: 12 个文档文件按实际目录同步，无残留字符串 API 示例
- [ ] E1-E5: 原计划文档 5 处事实错误已勘误
- [ ] `cargo check --workspace` 零 warning
- [ ] `cargo clippy --workspace -- -D warnings` 通过
- [ ] `cargo test --workspace` 全绿

---

## 七、假设与决策

| 决策项 | 选择 | 依据 |
|---|---|---|
| 不重做已完成项 | 跳过 Forms A/B/C、字符串 API 移除、bug 修复（find_by_id/foreign_key/set_property）、9 终端方法、ModelBuilder DSL 化 | 代码验证均已落地 |
| `single`/`all` 不引入 `QueryBuilder: Clone` | 保持现有 `take(2)` 实现 | 现有方案已正确工作，引入 Clone 是不必要的架构变更 |
| 不创建 `12-linq-terminals/` 目录 | 终端方法并入 05/06 章节 | 避免目录膨胀，原计划该提议基于错误的章节映射 |
| 文档章节映射按实际目录 | 弃用原计划 §4.6.3 的映射表 | 实际目录与计划误称完全不符（见 G4 表）|
| `split let` 风格降级为建议 | 不援引 project_memory | rust-ef 无该档案 |
| 原计划文档直接勘误而非重写 | 保留历史决策上下文，仅修事实错误 | 重写会丢失「为何这样设计」的决策记录 |

---

## 八、范围边界

**纳入本迭代**：G1（min/max 泛型化）、G2（DbValue TryFrom）、G3（两个测试文件）、G4（12 个文档同步）、E1-E5（原计划勘误）。

**不纳入（维持原计划 §5.3 的推迟项）**：
- `linq!` 类型推断（省略闭包类型标注）
- 子查询 / 关联过滤（`b.posts.any(p => ...)`）
- Lazy Loading
- 强类型元组投影（`select (b.id, b.title)` 返回 `(i32, String)`）
- `having` 嵌套表达式扩展（首版仅 `agg(col) op value`）

---

*本审查基于 2026-06-26 代码库实际状态，所有行号引用均经 Read/Grep 工具直接验证。原计划标注的「2026-06-25 状态」与实际不符，本迭代以实际代码为准。*

---

## 九、v2 补遗（2026-06-26 二次审查）

> 本节由 v2 迭代计划（`统一linq宏DSL改造计划_审查与迭代_v2_plan.md`）补全，记录 v1 漏判与范围低估。

### 9.1 v1 完成项对账

| v1 项 | v2 验证状态 |
|---|---|
| Phase 0 勘误（E1-E5） | ✅ 已落地（原计划文档已就地标注 `【勘误】`）|
| G1: min/max 泛型化 | ✅ 已落地（`query.rs:914/941`，`convert_aggregate_cell`@450）|
| G2: DbValue TryFrom | ✅ 已落地（`provider.rs` 有 `DbValueConvertError` + 8 类型 impl）|
| G3: 测试文件 | ⚠ 部分完成 → 由 G5 修复后转绿（18+17 全过）|
| G4: 文档同步 | ✗ v1 未启动，且范围被低估 → 由 G6 接管 |

### 9.2 v2 新发现的真实遗漏

**G5：`last()`/`last_or_default()` 无默认 PK 排序**（v1 漏判的代码缺陷）

- 位置：`crates/core/src/query.rs:1120-1133`（原 `last_or_default` 实现）
- Bug：当 `self.state.orderings` 为空时，反转循环是 no-op，`take(1)` 返回首行而非末行
- 与原计划偏差：原计划 §4 阶段 4 设计为「无显式排序时按 PK 倒序」，实际实现漏掉此关键步骤
- 影响：`linq_terminal_tests.rs` 中 `test_last_returns_entity` 与 `test_last_or_default_some_on_nonempty` 2 个测试失败
- **v2 修复**：在 `last_or_default` 反转循环前，若 `orderings.is_empty()` 则注入默认 PK DESC 排序（镜像 `find()` 的 PK 解析逻辑）。修复后 18+17 测试全绿。

**G6：文档同步范围扩展**（v1 G4 漏列 7 个文件）

- v1 G4 列出 12 个待同步文件，但 Grep 全工作区扫描发现额外 7 个文件也残留字符串 API
- v2 实际同步的文件清单（17 项）：
  - 主参考文档重写：`linq-macro.md`（补全 Form A/B/C）
  - grep 命中文件（10）：`eager-loading.md`/`many-to-many.md`/`first-crud.md`/`count-any.md`/`aggregation.md`/`common-pitfalls.md`/`performance-tips.md`/`crud-states.md`/`derive-attributes.md`/`crates/core/README.md`
  - 内容补充文件（6）：`filter-sort-page.md`/`group-by-having.md`/`join-queries.md`/`global-query-filters.md`/`execute-update.md`/`code-review-checklist.md`
  - 索引：`INDEX.md`（标题同步）
  - 根 `README.md`：`.order_by_desc("created_at")` → `linq!` 多子句形式
- 验证：Grep 全工作区扫描确认剩余命中均为「❌ 不要这样做」的教学示例或解释性文字，无真实残留

### 9.3 v2 实施结果

| v2 项 | 状态 |
|---|---|
| G5: `last_or_default` 默认 PK 排序 | ✅ 已修复（`query.rs:1126-1162`）|
| G6: 17 文件文档同步 | ✅ 已完成 |
| C: v2 补遗归档（本节）| ✅ 已追加 |
| `cargo test -p rust-ef --test linq_terminal_tests` | ✅ 18 passed |
| `cargo test -p rust-ef --test linq_dsl_tests` | ✅ 17 passed |
| Grep 全工作区字符串 API 残留 | ✅ 仅计划文档与教学「❌」示例 |

### 9.4 v2 范围边界

**纳入 v2**：G5（last 默认排序修复）、G6（17 文件文档同步，含 `linq-macro.md` 重写）、C（本 v2 补遗）

**不纳入（维持原计划 §5.3 与 v1 §八的推迟项）**：
- `linq!` 类型推断（省略闭包类型标注）
- 子查询 / 关联过滤（`b.posts.any(p => ...)`）
- Lazy Loading
- 强类型元组投影
- `having` 嵌套表达式扩展
- `QueryBuilder: Clone` 引入

---

*v2 补遗基于 2026-06-26 二次审查，G5 经 `last_or_default` 源码阅读 + 失败测试断言对照确认，G6 经全工作区 Grep 扫描确认。v1 已完成项经逐文件核对，v2 修复后所有相关测试全绿。*
