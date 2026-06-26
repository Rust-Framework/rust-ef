# 统一 `linq!` 宏 DSL 改造计划 — 审查与迭代 v3

> 版本: 对 `统一linq宏DSL改造计划_plan.md`（v0.4 Beta 1）、v1 迭代计划、v2 迭代计划的三次完整性审查
> 审查日期: 2026-06-26
> 审查方法: 逐项核对 v2 迭代计划验收清单 vs 当前代码/文档状态；扩展 Grep 模式以覆盖反引号包裹的字符串 API 残留

---

## 一、审查结论摘要

**核心结论**：v2 计划识别的 G5（last 默认排序）与 G6（17 文件文档同步）**主体已落地**，但 v2 §6.6 验收清单中的三项关键验证（`cargo clippy --workspace -- -D warnings` / `cargo test --workspace` / Grep 零残留）**均未执行**——v2 的 Task #9「最终验证」从未启动，导致以下三类真实遗漏在 v2 阶段未被捕获：

| 类别 | v2 状态 | v3 处置 |
|---|---|---|
| **G7: 文档残留**（v2 grep 模式漏判） | ✗ v2 grep 仅匹配 `\.method(`（带点前缀），漏掉反引号包裹的 `` `include_named` `` 与 INDEX 摘要中的 `set_column()` | **本迭代主体之一** |
| **G8: clippy 阻塞**（v2 未跑 clippy） | ✗ `entity.rs:111/312` 生成 `FK_DslBlog` 混合大小写常量，违反 `non_upper_case_globals`；阻塞 `cargo clippy --workspace -- -D warnings` | **本迭代主体之二** |
| **G9: 最终验证未执行**（v2 §6 验收清单未跑） | ✗ v2 Task #9 从未启动；`cargo test --workspace` / `cargo fmt --check` 状态未知 | **本迭代主体之三** |

v2 已落地的 G5（`last_or_default` 默认 PK 排序）与 G6（17 文件文档同步）经源码与 Read 验证为真，**不重做**。

---

## 二、v2 已完成项对账（无需再做）

经 Read/Grep 验证：

1. ✅ **G5: `last_or_default` 默认 PK 排序**：`crates/core/src/query.rs:1126-1162` 已注入 `ORDER BY <pk> DESC` 分支，镜像 `find()` 的 PK 解析逻辑；`single`/`single_or_default`/`all`/`contains`/`to_dictionary`/`long_count` 均正确实现。
2. ✅ **G6 主体**: 17 个文件经 Read 抽样验证（`linq-macro.md`/`execute-update.md`/`aggregation.md` 等）已改为 `linq!` 多子句形式。
3. ✅ **v2 补遗归档**: `.trae/documents/统一linq宏DSL改造计划_审查与迭代_plan.md` §九「v2 补遗」章节已追加。

---

## 三、v3 新发现的真实遗漏

### G7. 文档残留 —— v2 grep 模式漏判

**根因**：v2 §6.3 验证步骤的 Grep 模式均为 `\.method(`（带点前缀，匹配方法调用），漏掉两类场景：
1. 反引号包裹的方法名引用（如 `` `include_named` ``，无点前缀）
2. INDEX 摘要字段中的 API 调用签名（如 `execute_update().set_column().execute()`，被 v2 B4「INDEX 仅核对标题/路径」范围排除）

**残留清单（3 项）**：

| # | 文件 | 行 | 内容 | 性质 |
|---|---|---|---|---|
| 1 | `docs/rust-ef/04-relationships/one-to-many.md` | 66 | `> 注意：导航属性的物化（填充）需要通过 \`include_named\` 显式加载` | **真实残留**（非教学「❌」示例，是给用户的「这样做」指引）|
| 2 | `docs/rust-ef/08-bulk-operations/INDEX.md` | 7 | `\| [批量更新 ExecuteUpdate](execute-update.md) \| \`execute_update().set_column().execute()\` \|` | **真实残留**（INDEX 摘要引用已移除 API；目标文件已改但索引未同步）|
| 3 | `docs/rust-ef/INDEX.json` | 277 | `"summary": "execute_update().set_column().execute()"` | **真实残留**（同 #2，JSON 索引未同步）|

**对照**：v2 §6.3 grep 命中的其他文件（`linq-macro.md:221-222`、`common-pitfalls.md:100-102`、`code-review-checklist.md:19`、`crates/core/README.md:150`）经核验为**教学性 ❌ 示例或说明性文字**（描述「已移除的 API」），非残留，**不动**。

**修复方案**：
- #1: 改为 `> 注意：导航属性的物化（填充）需要通过 \`linq!(...; include b.x)\` 显式加载，参见 [Eager Loading](eager-loading.md)。`
- #2: 改为 `\| [批量更新 ExecuteUpdate](execute-update.md) \| \`linq!(...; set b.col, val; execute_update)\` \|`
- #3: 改为 `"summary": "linq!(...; set b.col, val; execute_update)"`

### G8. clippy 阻塞 —— `FK_<Type>` 常量违反 `non_upper_case_globals`

**现状**：`crates/macros/src/entity.rs` 两处生成 `FK_<Type>` 常量：
- 行 110-113: `format!("FK_{}", parent_type_name)` —— `parent_type_name` = 结构体名（如 `Blog`）
- 行 311-314: `format!("FK_{}", target)` —— `target` = `#[foreign_key(Target)]` 中的类型名（如 `DslBlog`）

`COLUMN_*`/`FIELD_*` 常量（行 358/378）已正确 `to_uppercase()`，但 `FK_*` 漏掉，导致生成 `pub const FK_DslBlog: &'static str = "blog_id";`，触发 `non_upper_case_globals` lint。

**影响**：
- `cargo clippy --workspace -- -D warnings` 失败
- 阻塞 v2 §6.1 验证步骤
- 在 `crates/core/tests/linq_dsl_tests.rs:38`（`#[foreign_key(DslBlog)]` 处）触发告警

**安全性核查**：Grep 全工作区搜索 `FK_[A-Z][a-z]` 零命中，说明 `FK_*` 常量**仅被宏内部通过 `stringify!(#fk_const)` 用于 fk_column_index 查表**（见 `entity.rs:169/320`），无任何外部引用。修改常量名或加 `#[allow]` 均安全。

**修复方案（推荐方案 A：加 `#[allow]` 属性）**：

在 `entity.rs` 两处 `quote!` 块中，为 `pub const #fk_ident: ...` 添加 `#[allow(non_upper_case_globals)]`：

```rust
// 行 316-318 改为：
fk_const_decls.push(quote! {
    #[allow(non_upper_case_globals)]
    pub const #fk_ident: &'static str = #col;
});

// 行 167 附近（BelongsTo/HasMany 分支中 FK 常量引用）无需改动，因为引用通过 #fk_const ident 而非字面字符串
```

**为何选 `#[allow]` 而非 `to_uppercase()`**：
- `to_uppercase()` 会把 `FK_DslBlog` 改为 `FK_DSLBLOG`，破坏 `stringify!(#fk_const)` 在错误信息/调试输出中的可读性
- `#[allow]` 是最小侵入修复，不影响任何现有调试行为
- 该 lint 是 style 类（非 correctness），`#[allow]` 是社区接受的常见处置

**备选方案 B**：`target.to_uppercase()` —— 同样安全但牺牲可读性，不推荐。

### G9. 最终验证未执行 —— v2 §6 验收清单的元遗漏

**现状**：v2 §6.1-6.3 列出三项必跑验证：
- `cargo check --workspace` / `cargo clippy --workspace -- -D warnings` / `cargo fmt --check`
- `cargo test --workspace` / `cargo test -p rust-ef --test linq_terminal_tests` / `linq_dsl_tests`
- Grep 全工作区字符串 API 残留零命中

v2 实施期间仅跑了 `linq_terminal_tests`（18 passed）与 `linq_dsl_tests`（17 passed），**未跑全工作区 clippy / fmt / test**。这是 v2 验收清单自身的元遗漏——验证步骤被列为 todo 但未执行。

**修复方案**：在 G7/G8 修复后，按 v2 §6 顺序完整跑一遍验证，确认：
- `cargo clippy --workspace -- -D warnings` 通过（G8 修复后预期通过）
- `cargo test --workspace` 全绿
- `cargo fmt --check` 通过
- Grep 扩展模式（含反引号形式 `` `include_named` `` 等）零残留

---

## 四、实施方案

### 阶段 A：修复 G8（entity.rs FK 常量 clippy 告警）

**文件**：`crates/macros/src/entity.rs`

**改动**：在两处 `quote!` 块的 `pub const #fk_ident: ...` 前加 `#[allow(non_upper_case_globals)]`：
- 行 316-318（`fk_const_decls.push`）：为 `pub const #fk_ident` 加属性
- 行 110-113 生成的 `#fk_const` ident 仅用于内部 `stringify!` 查表（行 169/320），**无需也无法**加 `#[allow]`（ident 本身不是 const 声明），保持现状即可

**注意**：仅第一处需要改。第二处（行 110-113）的 `fk_const` 是用于引用第一处生成的常量的 ident，不是新的常量声明。

**验证**：
```bash
cargo clippy -p rust-ef-macros -- -D warnings
cargo clippy -p rust-ef --tests -- -D warnings
# 重点确认 linq_dsl_tests.rs:38 的 FK_DslBlog 告警消失
```

### 阶段 B：修复 G7（3 个文档残留）

**文件清单与改动**：

#### B1. `docs/rust-ef/04-relationships/one-to-many.md` 行 66

```diff
- > 注意：导航属性的物化（填充）需要通过 `include_named` 显式加载，参见 [Eager Loading](eager-loading.md)。
+ > 注意：导航属性的物化（填充）需要通过 `linq!(...; include b.x)` 显式加载，参见 [Eager Loading](eager-loading.md)。
```

#### B2. `docs/rust-ef/08-bulk-operations/INDEX.md` 行 7

```diff
- | [批量更新 ExecuteUpdate](execute-update.md) | `execute_update().set_column().execute()` |
+ | [批量更新 ExecuteUpdate](execute-update.md) | `linq!(...; set b.col, val; execute_update)` |
```

#### B3. `docs/rust-ef/INDEX.json` 行 277

```diff
-       "summary": "execute_update().set_column().execute()",
+       "summary": "linq!(...; set b.col, val; execute_update)",
```

### 阶段 C：完成 G9（最终验证）

**前置条件**：阶段 A + B 已完成。

**执行步骤**：

```bash
# 1. 格式检查
cargo fmt --check

# 2. 编译与 lint
cargo check --workspace
cargo clippy --workspace -- -D warnings

# 3. 测试
cargo test --workspace
cargo test -p rust-ef --test linq_terminal_tests
cargo test -p rust-ef --test linq_dsl_tests
```

**Grep 扩展模式验证**（在 v2 §6.3 基础上增加反引号形式）：

```
# v2 原模式（带点前缀，匹配方法调用）
\.include_named\(
\.then_include_named\(
\.order_by\("
\.order_by_desc\("
\.sum\("
\.avg\("
\.min\("
\.max\("
\.group_by\(
\.having\(
\.select_columns\(
\.set_column\(
\.inner_join\(
\.left_join\(
find_by_id
filter_raw

# v3 新增模式（反引号包裹，匹配说明性引用——仅文档/教学 ❌ 示例允许）
`include_named`
`then_include_named`
`order_by_desc`
`set_column`
`select_columns`
`find_by_id`
`filter_raw`
```

期望命中均为教学性 ❌ 示例或说明性文字（`linq-macro.md:221/238`、`common-pitfalls.md:100`、`code-review-checklist.md:19`、`crates/core/README.md:150`、`filter-sort-page.md:3`），无真实残留。

---

## 五、实施顺序

```
阶段 A (G8 clippy 修复) ──► 阶段 B (G7 文档残留) ──► 阶段 C (G9 最终验证)
        │                          │
        └────── 可并行 ────────────┘
                                   │
                                   ▼
                           阶段 C 必须最后做
```

阶段 A 与 B 互相独立，可并行。阶段 C 必须在 A+B 完成后做（验证 A+B 的修复结果）。

---

## 六、验证步骤

### 6.1 编译与 lint（G8 重点）
```bash
cargo check --workspace
cargo clippy --workspace -- -D warnings   # G8 修复后预期通过
cargo fmt --check
```

### 6.2 测试
```bash
cargo test --workspace                     # 全工作区全绿
cargo test -p rust-ef --test linq_terminal_tests   # 期望 18 passed
cargo test -p rust-ef --test linq_dsl_tests        # 期望 17 passed
```

### 6.3 字符串 API 残留再确认（G7 重点）
用 v3 §四 阶段 C 中的扩展 Grep 模式扫描，期望命中均为教学性 ❌ 示例或说明性文字。

### 6.4 文档验证
- `docs/rust-ef/04-relationships/one-to-many.md:66` 已改为 `linq!(...; include b.x)` 形式
- `docs/rust-ef/08-bulk-operations/INDEX.md:7` 与 `INDEX.json:277` 摘要已改为 `linq!(...; set b.col, val; execute_update)` 形式

### 6.5 验收清单
- [ ] G7: `one-to-many.md:66` 残留已清除
- [ ] G7: `08-bulk-operations/INDEX.md:7` 摘要已同步
- [ ] G7: `INDEX.json:277` 摘要已同步
- [ ] G8: `entity.rs` FK 常量生成处加 `#[allow(non_upper_case_globals)]`
- [ ] G8: `cargo clippy --workspace -- -D warnings` 通过
- [ ] G9: `cargo fmt --check` 通过
- [ ] G9: `cargo test --workspace` 全绿
- [ ] G9: Grep 扩展模式扫描无真实残留（教学 ❌ 示例不计）
- [ ] C: v3 补遗归档到 v1/v2 计划文档

---

## 七、假设与决策

| 决策项 | 选择 | 依据 |
|---|---|---|
| G8 修复用 `#[allow]` 而非 `to_uppercase()` | 保留 `FK_DslBlog` 可读名 | 仅 style lint，社区接受 `#[allow]`；`to_uppercase()` 破坏 `stringify!` 调试输出 |
| G7 #1（one-to-many.md）属于真实残留 | 而非教学示例 | 该句是给用户的「这样做」指引（「需要通过 X 显式加载」），非 ❌ 反例 |
| G7 #2/#3（INDEX）属于真实残留 | 而非说明性文字 | 摘要字段描述章节内容，但章节内容已改、摘要未同步；用户按摘要操作会失败 |
| v2 其他 grep 命中（linq-macro.md / common-pitfalls.md / code-review-checklist.md / README.md）不动 | 教学性 ❌ 示例或描述性文字 | 内容明确标注「已移除」「❌」，是教学/说明用法 |
| 阶段 A/B 可并行 | 互不依赖 | A 改宏代码，B 改文档 |
| v3 补遗追加到 v2 文档而非 v1 | v2 已有「与 v1 的关系」章节，v3 紧随 v2 | 保持迭代脉络 |

---

## 八、范围边界

**纳入本 v3 迭代**：
- G7（3 个文档残留：one-to-many.md / INDEX.md / INDEX.json）
- G8（entity.rs FK 常量 clippy 告警修复，加 `#[allow]`）
- G9（最终验证：cargo clippy / test / fmt + Grep 扩展模式扫描）
- C（v3 补遗归档）

**不纳入（维持原计划 §5.3、v1 §八、v2 §八的推迟项）**：
- `linq!` 类型推断（省略闭包类型标注）
- 子查询 / 关联过滤（`b.posts.any(p => ...)`）
- Lazy Loading
- 强类型元组投影
- `having` 嵌套表达式扩展
- `QueryBuilder: Clone` 引入

---

## 九、与 v1/v2 迭代计划的关系

本 v3 计划**不替代** v1/v2，而是**补全** v2 验收清单未执行所暴露的三类遗漏：
- G7 = v2 grep 模式漏判的文档残留（3 项）
- G8 = v2 未跑 clippy 未发现的宏代码告警
- G9 = v2 §6 验收清单本身未执行

v1/v2 已完成项（G1/G2/Phase 0/G3/G4/G5/G6 主体）不重做。本 v3 仅修补 v2 的「最后一公里」。

执行顺序建议：
1. 阶段 A（G8 宏修复，1 行 `#[allow]` 添加）
2. 阶段 B（G7 文档残留，3 处编辑）
3. 阶段 C（G9 完整验证 + v3 补遗归档）

---

## 十、v3 补遗归档位置

完成本 v3 实施后，在 `.trae/documents/统一linq宏DSL改造计划_审查与迭代_v2_plan.md` 文末追加「v3 补遗」章节，注明：
- v2 主体（G5/G6）已完成对账
- G7（3 文档残留）+ G8（clippy 告警）+ G9（最终验证）已补全
- 验收清单全部 ✅

---

*本 v3 审查基于 2026-06-26 代码库与文档实际状态。G7 经扩展 Grep 模式（反引号 + INDEX 摘要）扫描确认；G8 经 `crates/macros/src/entity.rs:110-113/311-314` 源码阅读 + 全工作区 `FK_[A-Z][a-z]` 零外部引用核查确认；G9 经 v2 计划文档 Task #9 状态核对确认。*
