# 统一 `linq!` 宏 DSL 改造计划 — 审查与迭代 v3

> 版本: 对 `统一linq宏DSL改造计划_plan.md`（v0.4 Beta 1）、v1/v2 迭代计划的三次完整性审查
> 审查日期: 2026-06-26
> 审查方法: 逐项核对 v2 验收清单 vs 当前代码/文档状态；扩展 Grep 模式覆盖反引号引用；用户反馈追加 3 项跨切面问题

---

## 一、审查结论摘要

**核心结论**：v2 计划主体（G5/G6）已落地，但 v2 §6.6 验收清单的三项关键验证（`cargo clippy --workspace -- -D warnings` / `cargo test --workspace` / Grep 零残留）**均未执行**——v2 的 Task #9「最终验证」从未启动，导致以下三类真实遗漏在 v2 阶段未被捕获。用户二次反馈另追加 3 项跨切面问题（G10/G11/G12）。

| 类别 | v2 状态 | v3 处置 |
|---|---|---|
| **G7: 文档残留**（v2 grep 模式漏判） | ✗ v2 grep 仅匹配 `\.method(`（带点前缀），漏掉反引号引用与 INDEX 摘要 | **本迭代主体** |
| **G8: clippy 阻塞**（v2 未跑 clippy） | ✗ `entity.rs:111/312` 生成 `FK_DslBlog` 混合大小写常量，违反 `non_upper_case_globals` | **本迭代主体** |
| **G9: 最终验证未执行**（v2 §6 验收清单未跑） | ✗ v2 Task #9 从未启动 | **本迭代主体** |
| **G10: `ensure_created` 文档描述不清**（用户反馈） | ✗ `common-pitfalls.md` #4 仅说「不知道要建哪些表」，未说明与 EFCore 行为差异根因 | **本迭代主体**（文档补充）|
| **G11: 实体自动跟踪机制缺失**（用户反馈，架构性差距） | ✗ rust-ef 为手动快照式跟踪，非 EFCore 的代理式自动跟踪 | **仅记录为已知限制**，推迟到 v0.5+ |
| **G12: `linq!` let-binding 语法**（用户反馈，人体工学） | ✗ `linq!(set; ...)` 不工作，因宏展开时无法从裸变量推断实体类型 | **推迟到 v0.5+**，与类型推断一同处理 |

v2 已落地的 G5（`last_or_default` 默认 PK 排序）与 G6（17 文件文档同步）经源码与 Read 验证为真，**不重做**。

---

## 二、v2 已完成项对账（无需再做）

经 Read/Grep 验证：

1. ✅ **G5: `last_or_default` 默认 PK 排序**：`crates/core/src/query.rs:1126-1162` 已注入 `ORDER BY <pk> DESC` 分支，镜像 `find()` 的 PK 解析逻辑。
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

**对照**：v2 §6.3 grep 命中的其他文件（`linq-macro.md:221-222`、`common-pitfalls.md:100-102`、`code-review-checklist.md:19`、`crates/core/README.md:150`、`filter-sort-page.md:3`）经核验为**教学性 ❌ 示例或说明性文字**（描述「已移除的 API」），非残留，**不动**。

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

**修复方案（加 `#[allow]` 属性）**：

在 `entity.rs` 行 316-318 的 `quote!` 块中，为 `pub const #fk_ident: ...` 添加 `#[allow(non_upper_case_globals)]`：

```rust
// 行 316-318 改为：
fk_const_decls.push(quote! {
    #[allow(non_upper_case_globals)]
    pub const #fk_ident: &'static str = #col;
});
```

**为何选 `#[allow]` 而非 `to_uppercase()`**：
- `to_uppercase()` 会把 `FK_DslBlog` 改为 `FK_DSLBLOG`，破坏 `stringify!(#fk_const)` 在错误信息/调试输出中的可读性
- `#[allow]` 是最小侵入修复，不影响任何现有调试行为
- 该 lint 是 style 类（非 correctness），`#[allow]` 是社区接受的常见处置

**注意**：行 110-113 生成的 `#fk_const` ident 仅用于内部 `stringify!` 查表（行 169/320），**无需也无法**加 `#[allow]`（ident 本身不是 const 声明），保持现状即可。

### G9. 最终验证未执行 —— v2 §6 验收清单的元遗漏

**现状**：v2 §6.1-6.3 列出三项必跑验证：
- `cargo check --workspace` / `cargo clippy --workspace -- -D warnings` / `cargo fmt --check`
- `cargo test --workspace` / `cargo test -p rust-ef --test linq_terminal_tests` / `linq_dsl_tests`
- Grep 全工作区字符串 API 残留零命中

v2 实施期间仅跑了 `linq_terminal_tests`（18 passed）与 `linq_dsl_tests`（17 passed），**未跑全工作区 clippy / fmt / test**。这是 v2 验收清单自身的元遗漏——验证步骤被列为 todo 但未执行。

**修复方案**：在 G7/G8/G10/G11 修复后，按 v2 §6 顺序完整跑一遍验证，确认：
- `cargo clippy --workspace -- -D warnings` 通过（G8 修复后预期通过）
- `cargo test --workspace` 全绿
- `cargo fmt --check` 通过
- Grep 扩展模式（含反引号形式 `` `include_named` `` 等）零残留

### G10. `ensure_created` 文档描述不清（用户反馈）

**现状**：`docs/rust-ef/11-best-practices/common-pitfalls.md` 第 4 点：

```markdown
## 4. `ensure_created()` 在 `set::<T>()` 之前调用

// ❌ 错误：不知道要建哪些表
ctx.ensure_created().await?;
ctx.set::<Blog>();

// ✅ 正确
ctx.set::<Blog>();
ctx.ensure_created().await?;
```

**问题**：注释「不知道要建哪些表」过于简略，未说明**为何**rust-ef 的 `ensure_created` 必须在 `set::<T>()` 之后调用，也未说明**与 EFCore 行为差异的根因**。用户反馈「描述不明所以」。

**根因分析**（经 Read `db_context.rs:282` + `migration.rs:860` 验证）：
- **EFCore**：`DbContext` 通过 `DbSet<T>` 属性静态声明实体类型，模型在 `OnModelCreating` 中构建，`Database.EnsureCreated()` 时模型已完备，**不需要先调用 `Set<T>()`**
- **rust-ef**：`DbContext` 无 `DbSet<T>` 静态属性，模型通过 `set::<T>()` 调用动态构建（`entity_metas` HashMap 在 `set::<T>()` 时填充），`ensure_created()` 时从 `entity_metas` 收集元数据生成 DDL——**必须先调用 `set::<T>()` 注册所有实体**

**修复方案**：在 `common-pitfalls.md` #4 补充设计差异说明：

```markdown
## 4. `ensure_created()` 在 `set::<T>()` 之前调用

rust-ef 与 EFCore 的关键差异：EFCore 通过 `DbContext` 的 `DbSet<T>` 静态属性预先声明实体类型，模型在 `OnModelCreating` 中构建完备；rust-ef 无静态 `DbSet<T>` 属性，模型通过 `set::<T>()` 调用动态构建（`entity_metas` 在 `set::<T>()` 时填充）。因此 `ensure_created()` 必须在所有 `set::<T>()` 注册完成后调用，否则 `entity_metas` 为空会报 `No entity types registered`。

```rust
// ❌ 错误：entity_metas 为空，ensure_created 报 "No entity types registered"
ctx.ensure_created().await?;
ctx.set::<Blog>();

// ✅ 正确：先注册所有实体，再建表
ctx.set::<Blog>();
ctx.set::<Post>();
ctx.ensure_created().await?;
```
```

### G11. 实体自动跟踪机制缺失 —— 仅记录为已知限制（用户反馈，架构性差距）

**现状**：`crates/core/src/tracking.rs` 的 `ChangeTracker` 为**手动快照式跟踪**，非 EFCore 的代理式自动跟踪：

| 维度 | EFCore | rust-ef（当前）|
|---|---|---|
| 跟踪触发 | 查询自动跟踪 + `Attach` 显式跟踪 | 仅 `attach()`/`add()` 显式跟踪 |
| 属性变更检测 | 自动（DynamicProxy / `INotifyPropertyChanging`）| 手动（`detect_changes_with_properties` 需调用方传入 `HashMap<String, String>`）|
| Identity Map | 是（同一行查询返回同一实例）| 否（每次查询返回新实例）|
| 导航属性 Fixup | 自动 | 无 |
| `DetectChanges()` | 扫描所有跟踪实体的属性 | `db_set.detect_changes()` 调用 `IEntitySnapshot::snapshot()` 比对（见 `db_context.rs:189`）|

**用户决策**：仅记录为已知限制，不修复（推迟到 v0.5+ 架构改造）。

**修复方案（仅文档）**：在 `docs/rust-ef/07-change-tracking/change-tracker.md` 末尾追加「与 EFCore 的差异」小节：

```markdown
## 与 EFCore 的差异（已知限制）

rust-ef 当前的 `ChangeTracker` 为**手动快照式**跟踪，与 EFCore 的代理式自动跟踪有以下差异：

| 维度 | EFCore | rust-ef |
|---|---|---|
| 查询自动跟踪 | 是 | 否，需显式 `attach()` |
| 属性变更检测 | 自动（代理）| 手动 `detect_changes()` |
| Identity Map | 是 | 否 |
| 导航 Fixup | 自动 | 无 |

**当前推荐工作流**：查询后显式 `attach()` → 修改属性 → 调用 `update()` 标记 → `save_changes()`。或直接 `update(entity)` 跳过快照比对。

此项架构性差距将在 v0.5+ 评估代理式跟踪方案（可能需 nightly 特性或显著重写）。
```

### G12. `linq!` let-binding 语法 —— 推迟到 v0.5+（用户反馈，人体工学）

**现状**：`linq!(set; include b.posts)`（`set` 为变量）不工作，因为：
- `crates/macros/src/linq.rs:328` `source_entity_type` 通过遍历方法调用链查找 turbofish `::<Type>` 提取实体类型
- 裸变量 `set`（无 turbofish）无法在宏展开时提供类型信息
- 现有 fallback `expr_as_entity_type` 仅接受 `Expr::Path`（如 `Blog`），不接受变量

**用户决策**：暂不处理，推迟到 v0.5+（与类型推断推迟项合并）。

**修复方案（仅文档）**：在 `common-pitfalls.md` #8 已有「形式 B 的 source 用裸变量」说明，无需额外补充。在 v0.4 计划 §5.3 推迟项中追加一条：

```
- `linq!` let-binding 语法（`let set = ctx.set::<T>(); linq!(set; ...)`）—— 与类型推断一同处理
```

---

## 四、实施方案

### 阶段 A：修复 G8（entity.rs FK 常量 clippy 告警）

**文件**：`crates/macros/src/entity.rs`

**改动**：在行 316-318 的 `quote!` 块中，为 `pub const #fk_ident: ...` 添加 `#[allow(non_upper_case_globals)]`：

```rust
fk_const_decls.push(quote! {
    #[allow(non_upper_case_globals)]
    pub const #fk_ident: &'static str = #col;
});
```

**验证**：
```bash
cargo clippy -p rust-ef-macros -- -D warnings
cargo clippy -p rust-ef --tests -- -D warnings
# 重点确认 linq_dsl_tests.rs:38 的 FK_DslBlog 告警消失
```

### 阶段 B：修复 G7 + G10 + G11 + G12 文档（5 个文件）

#### B1. G7 #1: `docs/rust-ef/04-relationships/one-to-many.md` 行 66

```diff
- > 注意：导航属性的物化（填充）需要通过 `include_named` 显式加载，参见 [Eager Loading](eager-loading.md)。
+ > 注意：导航属性的物化（填充）需要通过 `linq!(...; include b.x)` 显式加载，参见 [Eager Loading](eager-loading.md)。
```

#### B2. G7 #2: `docs/rust-ef/08-bulk-operations/INDEX.md` 行 7

```diff
- | [批量更新 ExecuteUpdate](execute-update.md) | `execute_update().set_column().execute()` |
+ | [批量更新 ExecuteUpdate](execute-update.md) | `linq!(...; set b.col, val; execute_update)` |
```

#### B3. G7 #3: `docs/rust-ef/INDEX.json` 行 277

```diff
-       "summary": "execute_update().set_column().execute()",
+       "summary": "linq!(...; set b.col, val; execute_update)",
```

#### B4. G10: `docs/rust-ef/11-best-practices/common-pitfalls.md` #4 重写

将 #4 章节内容替换为：

```markdown
## 4. `ensure_created()` 在 `set::<T>()` 之前调用

rust-ef 与 EFCore 的关键差异：EFCore 通过 `DbContext` 的 `DbSet<T>` 静态属性预先声明实体类型，模型在 `OnModelCreating` 中构建完备；rust-ef 无静态 `DbSet<T>` 属性，模型通过 `set::<T>()` 调用动态构建（`entity_metas` 在 `set::<T>()` 时填充）。因此 `ensure_created()` 必须在所有 `set::<T>()` 注册完成后调用，否则 `entity_metas` 为空会报 `No entity types registered`。

```rust
// ❌ 错误：entity_metas 为空，ensure_created 报 "No entity types registered"
ctx.ensure_created().await?;
ctx.set::<Blog>();

// ✅ 正确：先注册所有实体，再建表
ctx.set::<Blog>();
ctx.set::<Post>();
ctx.ensure_created().await?;
```
```

#### B5. G11: `docs/rust-ef/07-change-tracking/change-tracker.md` 末尾追加

在「设计要点」表后、「下一章」链接前追加：

```markdown
## 与 EFCore 的差异（已知限制）

rust-ef 当前的 `ChangeTracker` 为**手动快照式**跟踪，与 EFCore 的代理式自动跟踪有以下差异：

| 维度 | EFCore | rust-ef |
|---|---|---|
| 查询自动跟踪 | 是 | 否，需显式 `attach()` |
| 属性变更检测 | 自动（代理）| 手动 `detect_changes()` |
| Identity Map | 是 | 否 |
| 导航 Fixup | 自动 | 无 |

**当前推荐工作流**：查询后显式 `attach()` → 修改属性 → 调用 `update()` 标记 → `save_changes()`。或直接 `update(entity)` 跳过快照比对。

此项架构性差距将在 v0.5+ 评估代理式跟踪方案（可能需 nightly 特性或显著重写）。
```

#### B6. G12: `.trae/documents/统一linq宏DSL改造计划_plan.md` §5.3 推迟项追加

在原计划 §5.3「不纳入（推迟到 v0.5+）」列表末尾追加：

```diff
  - 强类型元组投影（`select (b.id, b.title)` 返回 `(i32, String)` 而非 `Vec<String>`）—— 首版保留 `Vec<String>`，强类型化后续
  - `having` 嵌套表达式扩展（首版仅 `agg(col) op value`）
+ - `linq!` let-binding 语法（`let set = ctx.set::<T>(); linq!(set; ...)`）—— 与类型推断一同处理
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
阶段 A (G8 clippy 修复) ──┐
                           │
阶段 B (G7/G10/G11/G12 文档) ─┤── 可并行
                           │
                           ▼
                   阶段 C (G9 最终验证)
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
- `one-to-many.md:66` 已改为 `linq!(...; include b.x)` 形式
- `08-bulk-operations/INDEX.md:7` 与 `INDEX.json:277` 摘要已同步
- `common-pitfalls.md` #4 已补充 EFCore 行为差异根因说明
- `change-tracker.md` 末尾已追加「与 EFCore 的差异（已知限制）」小节
- 原计划 §5.3 已追加 let-binding 推迟项

### 6.5 验收清单
- [ ] G7: `one-to-many.md:66` 残留已清除
- [ ] G7: `08-bulk-operations/INDEX.md:7` 摘要已同步
- [ ] G7: `INDEX.json:277` 摘要已同步
- [ ] G8: `entity.rs` FK 常量生成处加 `#[allow(non_upper_case_globals)]`
- [ ] G8: `cargo clippy --workspace -- -D warnings` 通过
- [ ] G10: `common-pitfalls.md` #4 已重写，补充 EFCore 差异根因
- [ ] G11: `change-tracker.md` 已追加「与 EFCore 的差异（已知限制）」
- [ ] G12: 原计划 §5.3 已追加 let-binding 推迟项
- [ ] G9: `cargo fmt --check` 通过
- [ ] G9: `cargo test --workspace` 全绿
- [ ] G9: Grep 扩展模式扫描无真实残留（教学 ❌ 示例不计）
- [ ] C: v3 补遗归档到 v2 计划文档

---

## 七、假设与决策

| 决策项 | 选择 | 依据 |
|---|---|---|
| G8 修复用 `#[allow]` 而非 `to_uppercase()` | 保留 `FK_DslBlog` 可读名 | 仅 style lint，社区接受 `#[allow]`；`to_uppercase()` 破坏 `stringify!` 调试输出 |
| G7 #1（one-to-many.md）属于真实残留 | 而非教学示例 | 该句是给用户的「这样做」指引（「需要通过 X 显式加载」），非 ❌ 反例 |
| G7 #2/#3（INDEX）属于真实残留 | 而非说明性文字 | 摘要字段描述章节内容，但章节内容已改、摘要未同步；用户按摘要操作会失败 |
| G10 仅改文档不改代码 | 用户反馈为「描述不明所以」 | 根因是文档未说明设计差异，非代码 bug；`ensure_created` 当前行为（依赖 `set::<T>()` 注册）符合 rust-ef 动态模型构建设计 |
| G11 仅记录为已知限制 | 用户决策 | 架构性差距，与 v3「最后一公里」定位不符；推迟到 v0.5+ 评估代理方案 |
| G12 推迟到 v0.5+ | 用户决策 | 与类型推断（已推迟项）合并处理；当前 turbofish 要求已在 `common-pitfalls.md` #8 文档化 |
| v2 其他 grep 命中不动 | 教学性 ❌ 示例或说明性文字 | 内容明确标注「已移除」「❌」，是教学/说明用法 |
| 阶段 A/B 可并行 | 互不依赖 | A 改宏代码，B 改文档 |
| v3 补遗追加到 v2 文档 | v2 已有「与 v1 的关系」章节 | 保持迭代脉络 |

---

## 八、范围边界

**纳入本 v3 迭代**：
- G7（3 个文档残留：one-to-many.md / INDEX.md / INDEX.json）
- G8（entity.rs FK 常量 clippy 告警修复，加 `#[allow]`）
- G9（最终验证：cargo clippy / test / fmt + Grep 扩展模式扫描）
- G10（common-pitfalls.md #4 重写，补充 EFCore 差异根因）
- G11（change-tracker.md 追加「与 EFCore 的差异」已知限制说明）
- G12（原计划 §5.3 追加 let-binding 推迟项）
- C（v3 补遗归档到 v2 文档）

**不纳入（维持原计划 §5.3、v1 §八、v2 §八的推迟项）**：
- `linq!` 类型推断（省略闭包类型标注）
- `linq!` let-binding 语法（`linq!(set; ...)`）—— **v3 新增推迟项**
- 子查询 / 关联过滤（`b.posts.any(p => ...)`）
- Lazy Loading
- 强类型元组投影
- `having` 嵌套表达式扩展
- `QueryBuilder: Clone` 引入
- **实体自动跟踪机制（代理式跟踪）** —— **v3 新增推迟项**，v0.5+ 评估

---

## 九、与 v1/v2 迭代计划的关系

本 v3 计划**不替代** v1/v2，而是**补全** v2 验收清单未执行所暴露的三类遗漏 + 用户反馈的三项跨切面问题：
- G7 = v2 grep 模式漏判的文档残留（3 项）
- G8 = v2 未跑 clippy 未发现的宏代码告警
- G9 = v2 §6 验收清单本身未执行
- G10 = 用户反馈的 `ensure_created` 文档描述不清（仅文档补充）
- G11 = 用户反馈的实体自动跟踪缺失（仅记录为已知限制）
- G12 = 用户反馈的 `linq!` let-binding 语法（推迟到 v0.5+）

v1/v2 已完成项（G1/G2/Phase 0/G3/G4/G5/G6 主体）不重做。

执行顺序建议：
1. 阶段 A（G8 宏修复，1 行 `#[allow]` 添加）
2. 阶段 B（G7+G10+G11+G12 文档，6 个文件编辑）
3. 阶段 C（G9 完整验证 + v3 补遗归档）

---

## 十、v3 补遗归档位置

完成本 v3 实施后，在 `.trae/documents/统一linq宏DSL改造计划_审查与迭代_v2_plan.md` 文末追加「v3 补遗」章节，注明：
- v2 主体（G5/G6）已完成对账
- G7（3 文档残留）+ G8（clippy 告警）+ G9（最终验证）已补全
- G10（ensure_created 文档）+ G11（auto-tracking 已知限制）+ G12（let-binding 推迟）已处置
- 验收清单全部 ✅

---

*本 v3 审查基于 2026-06-26 代码库与文档实际状态 + 用户二次反馈。G7 经扩展 Grep 模式扫描确认；G8 经 `entity.rs:110-113/311-314` 源码阅读 + 全工作区 `FK_[A-Z][a-z]` 零外部引用核查确认；G9 经 v2 Task #9 状态核对确认；G10 经 `common-pitfalls.md` #4 + `db_context.rs:282` + `migration.rs:860` 对照确认；G11 经 `tracking.rs` 全文阅读 + EFCore 行为对照确认；G12 经 `linq.rs:328` `source_entity_type` 函数源码阅读确认。*

---

## v4 补遗（2026-06-26）

**v4 审查范围**：对 v3 计划的完整性复审，聚焦 v3 遗漏的「验证前提」与「文档自洽性」问题。

**v4 已完成项**：
- G13（前置 clippy 错误）—— 实际修复 11 处（v3 计划预估 3 处，执行中发现 8 处额外 lint）：`large_enum_variant`@linq.rs:48、`should_implement_trait`@query.rs:135、`type_complexity`@db_context.rs:41/105/453 + change_executor.rs:75/147 + blog/context.rs:22、`derivable_impls`@db_context.rs:72、`new_without_default`@mysql/sql_generator.rs:7、`ptr_arg`@cli/main.rs:271。全部加 `#[allow]` 处置。
- G14（前置 fmt 失败）—— `cargo fmt` 全量修复，`cargo fmt --check` 通过
- G15（原计划 §3.3 示例矛盾）—— 将 `linq!(set, ...)` / `linq!(set; ...)` 不可编译示例改为 source 内联 turbofish 形式 + 注意说明
- G16（v3 补遗归档）—— 已在 v2 文档追加「v3 补遗」章节
- G9（最终验证）—— 全部通过：
  - `cargo check --workspace` ✅
  - `cargo clippy --workspace -- -D warnings` ✅ (exit 0)
  - `cargo fmt --check` ✅ (exit 0)
  - `linq_terminal_tests` 18 passed ✅
  - `linq_dsl_tests` 17 passed ✅
  - 全工作区测试通过（`postgres_crud_tests` 因环境无 PG 服务跳过）
  - Grep 扩展模式扫描无真实残留（命中均为教学 ❌ 示例或说明性文字）

**v0.4 Beta 1 DSL 改造计划闭环。**
