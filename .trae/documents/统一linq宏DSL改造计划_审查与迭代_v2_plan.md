# 统一 `linq!` 宏 DSL 改造计划 — 审查与迭代 v2

> 版本: 对 `统一linq宏DSL改造计划_plan.md`（v0.4 Beta 1）与 v1 迭代计划（`统一linq宏DSL改造计划_审查与迭代_plan.md`）的二次完整性审查
> 审查日期: 2026-06-26
> 审查方法: 逐项核对 v1 迭代计划主张 vs 当前代码状态，并用 Grep 全工作区扫描字符串 API 残留

---

## 一、审查结论摘要

**核心结论**：v1 迭代计划识别的 4 个真实遗漏（G1-G4）中，G1/G2/Phase 0 已落地，G3 因 `last()` 实现缺陷仍有 2 个失败测试，G4 完全未启动且**范围被低估**——v1 列出 12 个待同步文件，但 Grep 全工作区扫描发现额外 7 个文件也残留字符串 API，共需同步 16+ 个文件。同时本次审查发现 **1 个 v1 漏判的真实代码缺陷 G5**：`last()`/`last_or_default()` 在无显式排序时不添加默认 PK 排序，违反原计划 §4 阶段 4 的设计意图，是 2 个失败测试的根因。

| 类别 | v1 状态 | v2 处置 |
|---|---|---|
| Phase 0 勘误（E1-E5） | ✅ 已完成 | 仅对账 |
| G1: min/max 泛型化 | ✅ 已完成（`query.rs:914/941`，`convert_aggregate_cell`@450） | 仅对账 |
| G2: DbValue TryFrom | ✅ 已完成（`provider.rs` 已有 `DbValueConvertError` + 8 类型 impl） | 仅对账 |
| G3: 测试文件 | ⚠ 部分完成（`linq_dsl_tests` 17 全绿；`linq_terminal_tests` 18 中 2 失败） | 由 G5 修复后验证 |
| G4: 文档同步 | ✗ 未启动，且范围被低估（v1 列 12 文件，实际需 16+） | **本迭代主体之一**（G6 扩展） |
| **G5: `last()` 默认排序缺失**（v1 漏判） | ✗ 真实 bug，根因于实现偏离原设计 | **本迭代主体之二** |

---

## 二、v1 已完成项对账（无需再做）

经逐文件 Read 验证：

1. ✅ **E1-E5 勘误**：原 `统一linq宏DSL改造计划_plan.md` 已就地标注 5 处 `【勘误】`（宏数量 3 非 13、行号更新、`field_const` 函数名、`FilterCondition::new` 签名、project_memory 援引降级）。
2. ✅ **G1 min/max 泛型化**：`crates/core/src/query.rs:914` (`min_internal<V>`) 与 `:941` (`max_internal<V>`) 均为泛型 `V: TryFrom<DbValue, Error = DbValueConvertError>`，返回 `EFResult<Option<V>>`；辅助函数 `convert_aggregate_cell<V>`@450 处理 SQL NULL 与 `String("NULL")` 边界。
3. ✅ **G2 DbValue TryFrom**：`crates/core/src/provider.rs` 已定义 `DbValueConvertError`（含 `source: DbValue` + `target_type: &'static str`）+ `From<DbValueConvertError> for EFError`，并为 `i32/i64/f64/f32/String/bool/Vec<u8>/i16` 实现 `TryFrom<DbValue>`（native 变体直取 + `DbValue::String(s)` 走 `s.parse()`/`to_string()`）。
4. ✅ **`find_by_id` bug 修复**：`query.rs:665` 已实现 `pub async fn find(self, id: impl Into<DbValue>) -> EFResult<Option<T>>`，使用 `T::entity_meta().primary_keys.first()` 取 PK 列名，不再硬编码 `"id"`。
5. ✅ **`#[foreign_key]` bug 修复**：`entity.rs:655` `extract_foreign_key_field_name` 返回 `quote!{ None }`，FK 由 `NavigationMeta` 默认推导。
6. ✅ **`set_property` 死代码移除**：`entity.rs` derive 中已无该 accessor 生成代码。

---

## 三、v2 新发现的真实遗漏

### G5. `last()`/`last_or_default()` 无默认 PK 排序 —— v1 漏判的代码缺陷

**现状**：`crates/core/src/query.rs:1120-1133`，`last_or_default` 实现：

```rust
pub async fn last_or_default(mut self) -> EFResult<Option<T>>
where T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
{
    // Reverse all orderings to get the "last" row.
    for o in &mut self.state.orderings {
        o.direction = match o.direction {
            OrderDirection::Ascending => OrderDirection::Descending,
            OrderDirection::Descending => OrderDirection::Ascending,
        };
    }
    let mut results = self.take(1).to_list().await?;
    Ok(results.pop())
}
```

**Bug**：当 `self.state.orderings` 为空（调用方未显式 `order_by`）时，反转循环是 no-op，`take(1)` 返回的是默认 DB 顺序下的**第一行**而非最后一行。

**与原计划偏差**：原计划 §4 阶段 4 明确设计为：

```rust
pub async fn last(mut self) -> EFResult<T> {
    let pk = T::entity_meta().primary_keys.first()
        .ok_or_else(|| EFError::Query("last requires primary key".into()))?;
    self = self.order_by_desc_column_internal(pk.column_name);
    self.first().await
}
```

即「无显式排序时按 PK 倒序」，但实际实现仅做反转，**漏掉了「无排序时插入默认 PK DESC」**这一关键步骤。这是 v1 审查未识别的真实代码缺陷，导致 `linq_terminal_tests.rs` 中 2 个测试失败：

| 测试 | 期望 | 实际 | 原因 |
|---|---|---|---|
| `test_last_returns_entity`（line 47-52） | `name == "i2"`（最高 id） | `"i0"`（最低 id） | 无排序 → take(1) 取首行 |
| `test_last_or_default_some_on_nonempty`（line 73-78） | `name == "i1"`（最高 id） | `"i0"`（最低 id） | 同上 |

**修复方案**：在 `last_or_default` 反转循环前，若 `self.state.orderings.is_empty()` 则注入默认 PK DESC 排序。需要 `T: IGetKeyValues`（已有 trait bound）+ `T::entity_meta()` 返回 PK 列名。

```rust
pub async fn last_or_default(mut self) -> EFResult<Option<T>>
where
    T: IFromRow + INavigationSetter + IGetKeyValues + IEntitySnapshot,
{
    // 无显式排序时，按主键倒序以确定「最后一条」语义。
    // 主键缺失则报错（与原计划设计一致）。
    if self.state.orderings.is_empty() {
        let meta = T::entity_meta();
        let pk_col = meta.primary_keys.first().map(|s| s.as_ref())
            .or_else(|| meta.properties.iter()
                .find(|p| p.is_primary_key)
                .map(|p| p.column_name.as_ref()))
            .ok_or_else(|| crate::error::EFError::Query(
                "last_or_default requires a primary key when no explicit ordering is set".into()
            ))?;
        self.state.orderings.push(OrderBy::new(pk_col, OrderDirection::Descending));
    } else {
        // 有显式排序时，反转方向。
        for o in &mut self.state.orderings {
            o.direction = match o.direction {
                OrderDirection::Ascending => OrderDirection::Descending,
                OrderDirection::Descending => OrderDirection::Ascending,
            };
        }
    }
    let mut results = self.take(1).to_list().await?;
    Ok(results.pop())
}
```

`last()` 无需改动（它委托 `last_or_default`）。

**风险**：现有调用方若依赖「无排序时 last 返回任意行」的非确定行为，会因修复后确定性变化而行为改变。但原计划设计本就要求确定性，且测试期望也是确定性的，故此修复符合设计意图。

### G6. 文档同步范围被低估 —— v1 G4 漏列 7 个文件

**现状**：v1 G4 列出 12 个待同步文件（`linq-macro.md`/`filter-sort-page.md`/`dbset-and-queryable.md`/`count-any.md`/`aggregation.md`/`group-by-having.md`/`join-queries.md`/`global-query-filters.md`/`eager-loading.md`/`execute-update.md`/`code-review-checklist.md`/`INDEX.md`+`INDEX.json`/`README.md`），但全工作区 Grep 搜索 `\.include_named\(|\.order_by\("|\.sum\("|find_by_id|filter_raw` 发现**额外 7 个文件**也残留字符串 API：

| # | 文件 | 命中数 | 残留内容示例 |
|---|---|---|---|
| 1 | `docs/rust-ef/04-relationships/eager-loading.md` | 3 | `.include_named("posts")` ×3 + `.then_include_named("comments")` ×2 |
| 2 | `docs/rust-ef/04-relationships/many-to-many.md` | 1 | `find_by_id` 等 |
| 3 | `docs/rust-ef/02-quickstart/first-crud.md` | 2 | `find_by_id` 等 |
| 4 | `docs/rust-ef/05-query-patterns/count-any.md` | 2 | `.sum("` 等 |
| 5 | `docs/rust-ef/06-advanced-query/aggregation.md` | 2 | `.sum("` 等 |
| 6 | `docs/rust-ef/11-best-practices/common-pitfalls.md` | 2 | 字符串 API 残留 |
| 7 | `docs/rust-ef/11-best-practices/performance-tips.md` | 1 | 字符串 API 残留 |
| 8 | `docs/rust-ef/07-change-tracking/crud-states.md` | 1 | `find_by_id` |
| 9 | `docs/rust-ef/03-entity-design/derive-attributes.md` | 1 | 字符串 API 残留 |
| 10 | `crates/core/README.md` | 2 | `.include_named("posts")` + `.sum("col")` |

**另外**，`docs/rust-ef/05-query-patterns/linq-macro.md`（v1 G4 列出但未标注为「需大改」）经 Read 验证**严重过时**：仅文档化 Form A（过滤闭包）与 legacy `=>` 排序语法，完全未提 Form B（多子句）/Form C（值产生）。这是 `linq!` 的**主参考文档**，必须重写。

**修复方案**：将 v1 G4 的同步范围从 12 文件扩展为 16 文件（v1 的 12 + 新增 7 - 重叠后去重）+ 1 个 README + INDEX。完整清单见下文「四、实施方案」。

---

## 四、实施方案

### 阶段 A：修复 G5（`last_or_default` 默认排序）

**目标**：修复 2 个失败测试，使 `linq_terminal_tests.rs` 全绿。

**文件**：`crates/core/src/query.rs`

**改动**：仅修改 `last_or_default` 方法体（line 1120-1133），在反转循环前增加「无排序时注入默认 PK DESC」分支。`last()` 与 `single()`/`single_or_default()`/`all()`/`contains()`/`to_dictionary()`/`long_count()` 均无需改动。

**验证**：
```bash
cargo test -p rust-ef --test linq_terminal_tests
# 期望：18 passed, 0 failed
cargo test -p rust-ef --test linq_dsl_tests
# 期望：17 passed, 0 failed（G5 修复不应影响 DSL 测试）
```

### 阶段 B：文档同步（G6，扩展范围）

**目标**：将 `docs/rust-ef/` 与 `crates/core/README.md` 下所有字符串 API 示例改为 `linq!` 形式，并为 `linq-macro.md` 补全 Form B/C 文档。

**完整文件清单（17 项）**：

#### B1. 主参考文档重写（高优先级）
- **`docs/rust-ef/05-query-patterns/linq-macro.md`** — 当前仅 Form A，需补全：
  - Form A（过滤闭包，保持现有内容）
  - Form B（多子句查询）语法总览 + 全部 17 个子句表
  - Form C（`filter`/`index`/`key` 值产生）用法
  - 移除 legacy `=>` 排序语法说明，改为 `order_by` 子句
  - 推荐代码风格（split `let` bindings 作为建议，非硬约束）

#### B2. 已确认残留字符串 API 的文件（10 项，按 grep 命中）
- `docs/rust-ef/04-relationships/eager-loading.md` — `.include_named("posts")` → `linq!(include b.posts then b.comments)`
- `docs/rust-ef/04-relationships/many-to-many.md` — `find_by_id` 等 → `find(id)` / `linq!`
- `docs/rust-ef/02-quickstart/first-crud.md` — `find_by_id` → `find(id)`
- `docs/rust-ef/05-query-patterns/count-any.md` — `.sum("`/字符串 API → `linq!(sum b.col)`/`linq!(count)`
- `docs/rust-ef/06-advanced-query/aggregation.md` — `.sum("` 等 → `linq!(sum/avg/min/max b.col)`，并补充 G1 修复后的泛型返回示例（`let v: i64 = linq!(set; max b.col).await?.unwrap_or(0);`）
- `docs/rust-ef/11-best-practices/common-pitfalls.md` — 字符串 API 残留 → `linq!` 形式
- `docs/rust-ef/11-best-practices/performance-tips.md` — 同上
- `docs/rust-ef/07-change-tracking/crud-states.md` — `find_by_id` → `find(id)`
- `docs/rust-ef/03-entity-design/derive-attributes.md` — 字符串 API 残留 → `linq!` 形式
- `crates/core/README.md` — `.include_named("posts")`/`.sum("col")` → `linq!` 形式

#### B3. v1 G4 列出但 grep 未命中的文件（6 项，需内容补充而非替换）
这些文件无字符串 API 残留，但需补充 Form B/C 的新语法示例：
- `docs/rust-ef/05-query-patterns/filter-sort-page.md` — 补充 `order_by`/`take`/`skip` 子句示例
- `docs/rust-ef/05-query-patterns/dbset-and-queryable.md` — 补充 `linq!(ctx.set::<T>(); ...)` 形式
- `docs/rust-ef/06-advanced-query/group-by-having.md` — 补充 `group_by`/`having` 子句示例
- `docs/rust-ef/06-advanced-query/join-queries.md` — 补充 `inner_join`/`left_join` 多参数闭包示例
- `docs/rust-ef/06-advanced-query/global-query-filters.md` — 补充 `linq!(filter |b| ...)` Form C 示例
- `docs/rust-ef/08-bulk-operations/execute-update.md` — 补充 `set` + `execute_update` 子句示例
- `docs/rust-ef/11-best-practices/code-review-checklist.md` — 补充「统一 `linq!` 风格」检查项

#### B4. 索引与根 README
- `docs/rust-ef/INDEX.md` + `INDEX.json` — 章节标题/路径核对（v1 已指出实际目录结构正确，仅需确认无新增/删除章节）
- 根 `README.md` — Best Practices 章节示例同步（若有 `linq!` 相关示例）

**改动原则**：
- 仅改示例代码与说明文字，不动目录结构
- 不创建 `12-linq-terminals/`（v1 已决策：终端方法并入 `05-query-patterns/count-any.md` 与 `06-advanced-query/aggregation.md`）
- `split let` 风格作为建议，不援引 project_memory
- 代码块经 `cargo test --doc` 或 `rustc --edition 2021 --crate-type lib` 编译验证（至少 linq-macro.md 的示例）

### 阶段 C：v2 迭代计划自身同步

**目标**：在 v1 迭代计划文档中追加 v2 审查结论，避免后续执行者重复审查。

**文件**：`.trae/documents/统一linq宏DSL改造计划_审查与迭代_plan.md`

**改动**：在文末追加「v2 补遗」章节，注明：
- G1/G2/Phase 0 已完成（对账结论）
- G3 阻塞于 G5，待 G5 修复后验证
- G4 范围扩展为 G6（17 文件）
- 新增 G5（last 默认排序 bug）

---

## 五、实施顺序

```
阶段 A (G5 修复) ──► 验证 linq_terminal_tests 全绿
        │
        ▼
阶段 B (G6 文档同步, 17 文件) ──► 可与阶段 A 并行（不依赖代码修复）
        │
        ▼
阶段 C (v2 补遗同步到 v1 文档)
        │
        ▼
最终验证 (cargo test --workspace + clippy + fmt + grep 零残留)
```

阶段 A 与阶段 B 互相独立，可并行；阶段 C 必须在 A/B 完成后做（记录最终状态）。

---

## 六、验证步骤

### 6.1 编译与 lint
```bash
cargo check --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

### 6.2 测试（重点验证 G5 修复）
```bash
cargo test -p rust-ef --test linq_terminal_tests   # 期望 18 passed
cargo test -p rust-ef --test linq_dsl_tests         # 期望 17 passed
cargo test --workspace                              # 全工作区全绿
```

### 6.3 字符串 API 残留再确认（重点验证 G6）
```bash
# 用 Grep 搜索以下模式，期望全零命中（仅计划/历史文档可保留作对照）：
#   \.include_named\(
#   \.then_include_named\(
#   \.order_by\("
#   \.order_by_desc\("
#   \.sum\("
#   \.avg\("
#   \.min\("
#   \.max\("
#   \.group_by\(
#   \.having\(
#   \.select_columns\(
#   \.set_column\(
#   \.inner_join\(
#   \.left_join\(
#   find_by_id
#   filter_raw
#   \.include_named\(
```

### 6.4 文档验证
- `docs/rust-ef/05-query-patterns/linq-macro.md` 包含 Form A/B/C 三类语法说明
- 所有文档代码块无字符串 API 残留
- `INDEX.md`/`INDEX.json` 章节标题与文件路径一致

### 6.5 v2 补遗验证
重新读取 `.trae/documents/统一linq宏DSL改造计划_审查与迭代_plan.md`，确认文末有「v2 补遗」章节。

### 6.6 验收清单
- [ ] G5: `last_or_default` 在无显式排序时注入默认 PK DESC，`linq_terminal_tests` 18 全绿
- [ ] G6: `linq-macro.md` 补全 Form B/C 文档
- [ ] G6: 10 个 grep 命中文件已清除字符串 API 残留
- [ ] G6: 6 个未命中文件已补充 Form B/C 示例
- [ ] G6: `crates/core/README.md` 同步
- [ ] G6: `INDEX.md`/`INDEX.json` 核对
- [ ] G6: 根 `README.md` 同步（若有相关示例）
- [ ] C: v1 迭代计划文档追加「v2 补遗」章节
- [ ] `cargo check --workspace` 零 warning
- [ ] `cargo clippy --workspace -- -D warnings` 通过
- [ ] `cargo test --workspace` 全绿
- [ ] Grep 全工作区字符串 API 残留零命中（除计划/历史文档）

---

## 七、假设与决策

| 决策项 | 选择 | 依据 |
|---|---|---|
| G5 修复采用「无排序时注入默认 PK DESC」 | 而非「修复测试期望」 | 原计划 §4 阶段 4 设计本就如此；测试期望符合语义；保持确定性 |
| G5 主键缺失时报错而非静默返回首行 | `EFError::Query` | 与 `find()` 一致；原设计要求 PK |
| G6 文档同步扩展到 17 文件 | 而非 v1 的 12 文件 | Grep 实证 7 个额外文件有残留 |
| G6 不创建 `12-linq-terminals/` 目录 | 终端方法并入 05/06 章节 | 维持 v1 决策，避免目录膨胀 |
| G6 `linq-macro.md` 需重写而非补充 | 当前仅 Form A，缺 Form B/C | 该文件是 `linq!` 主参考文档 |
| v2 补遗追加到 v1 文档而非新建 | 保留单一迭代计划脉络 | 避免文档碎片化 |
| 阶段 A/B 可并行 | 文档改动不依赖代码修复 | 互不阻塞 |

---

## 八、范围边界

**纳入本 v2 迭代**：
- G5（`last_or_default` 默认 PK 排序修复）
- G6（17 文件文档同步，含 `linq-macro.md` 重写）
- C（v2 补遗同步到 v1 迭代计划文档）

**不纳入（维持原计划 §5.3 与 v1 §八的推迟项）**：
- `linq!` 类型推断（省略闭包类型标注）
- 子查询 / 关联过滤（`b.posts.any(p => ...)`）
- Lazy Loading
- 强类型元组投影（`select (b.id, b.title)` 返回 `(i32, String)` 而非 `Vec<String>`）
- `having` 嵌套表达式扩展（首版仅 `agg(col) op value`）
- `QueryBuilder: Clone` 引入（现有 `take(2)` + `to_list` 方案已正确工作）

---

## 九、与 v1 迭代计划的关系

本 v2 计划**不替代** v1，而是**补全** v1 漏判的 G5 + 扩展 v1 低估的 G4 范围（升级为 G6）。v1 已完成的 G1/G2/Phase 0 不重做。v1 的 G3（测试文件）在 G5 修复后自动满足（2 失败测试转绿）。v1 的 G4 由 G6 接管并扩展。

执行顺序建议：
1. 先做 G5（小改动，1 个方法体，立即解锁 G3 验证）
2. 再做 G6（17 文件文档同步，工作量较大但不阻塞代码）
3. 最后做 C（v2 补遗归档）

---

*本 v2 审查基于 2026-06-26 代码库实际状态，所有行号引用均经 Read/Grep 工具直接验证。v1 已完成项经逐文件核对确认，G5 经 `last_or_default` 源码阅读 + 失败测试断言对照确认，G6 经全工作区 Grep 扫描确认。*

---

## v3 补遗（2026-06-26）

**v3 审查范围**：对 v2 验收清单未执行所暴露的遗漏 + 用户二次反馈的 3 项跨切面问题。

**v3 已完成项**：
- G7（3 文档残留：one-to-many.md / INDEX.md / INDEX.json）—— 已修复
- G8（entity.rs FK 常量 clippy 告警）—— 已加 `#[allow(non_upper_case_globals)]`
- G10（common-pitfalls.md #4 重写）—— 已补充 EFCore 差异根因
- G11（change-tracker.md 已知限制）—— 已追加「与 EFCore 的差异」
- G12（let-binding 推迟项）—— 已追加到原计划 §5.3

**v3 未完成的验证**（由 v4 接管）：
- G9（最终验证）：因前置 clippy/fmt 错误（v4 G13/G14）未执行
- v3 补遗归档：本节即补遗

**v4 接续**：见 `统一linq宏DSL改造计划_审查与迭代_v4_plan.md`，处理 G13/G14/G15 + 完成 G9 验证。
