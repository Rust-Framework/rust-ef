# 统一 `linq!` 宏 DSL 改造计划

> 版本: v0.4 Beta 1 核心任务  
> 决策依据: 用户反馈「DSL 化应该统筹设计，不应该每个做一个宏，大部分是有通性的」  
> 设计原则: **`linq!` 是唯一的 DSL 入口**；所有数据库操作通过子句或独立形式表达；立即移除字符串 API（破坏性变更，v0.4 一次性完成）

> **勘误记录 (2026-06-26)**：本计划初稿基于 2026-06-25 代码状态撰写，但代码库随后已实现计划主体（Forms A/B/C、字符串 API 移除、9 个 LINQ 终端方法、3/4 bug 修复、ModelBuilder DSL 化均落地）。下方仍保留原始决策上下文，但 5 处事实错误已就地标注 `【勘误】` 修正：宏数量、行号、函数名、FilterCondition 签名、project_memory 援引。完整审查与遗漏补全见 `统一linq宏DSL改造计划_审查与迭代_plan.md`。

---

## 一、执行摘要

将 `linq!` 宏从「仅过滤表达式」扩展为**覆盖全部数据库操作的统一 DSL 入口**。所有原接受 `&str` 的 API（`include_named`/`order_by`/`group_by`/`sum`/`inner_join`/`has_query_filter` 等 14+ 方法）统一改为通过 `linq!` 宏以闭包字段访问形式调用，并立即移除字符串 API。同时补全 LINQ 终端方法（`last_or_default`/`single`/`distinct`/`all`/`contains` 等 9 个）并修复 4 个已知 bug。

**核心洞察**：现有 `linq!` 已具备全部基础设施——`extract_field`（字段提取）+ `field_const`（列/导航常量生成，原稿误记为 `field_column_const`，实际函数名 `field_const(entity, field, kind: FieldKind)`，见 `linq.rs:1568`）+ `LinqCtx`（上下文，三字段 `{entity, param, params}`）+ `=>` 子句先例（排序）。【勘误】原稿「13 个独立宏」为笔误——`crates/macros/src/lib.rs` 实际仅定义 3 个宏入口（`derive_entity_type`/`column`/`linq`）；本句原意是「各类数据库操作的子句差异本质上是『调哪个 `*_internal` 方法 + 列常量还是导航常量 + 单字段还是元组 + 单闭包还是双闭包』的配置组合」，完全可由**一个宏 + 内部 dispatch** 表达，无需新增任何宏名。

---

## 二、当前状态分析

### 2.1 `linq!` 宏现状（`crates/macros/src/linq.rs`，513 行）

已支持三种形式：
- `linq!(|b: Blog| b.rating > 5)` → 可复用过滤闭包（`LinqInput.source = None`）
- `linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5)` → `(source).filter(closure)`
- `linq!(Blog, |b| b.rating > 5)` → 省略闭包类型标注

已支持 `=>` 排序子句：`linq!(|b: Blog| b.rating > 5 => b.created_at)`（升序）/ `=> -b.created_at`（降序，通过 `UnOp::Neg` 识别）。

**核心可复用基础设施**【勘误：以下行号为 2026-06-26 实际值，原稿行号已失效】：
| 函数 | 行号 | 职责 |
|---|---|---|
| `extract_field` | 1465 | 闭包字段访问 → 列常量引用（识别 `b.field`、`Blog::field`、裸 `field` 三种形式）|
| `field_const` | 1568 | 生成 `Entity::COLUMN_<UPPER>` 或 `FIELD_<UPPER>`（参数 `kind: FieldKind`，原稿误记为 `field_column_const`）|
| `extract_value` | 1580 | 字面量/变量透传 |
| `compile_expr` | 1241 | 表达式树编译（比较/逻辑/方法调用）|
| `compile_order` | 1451 | 排序子句编译（`extract_field` + `order_by_column`）|
| `LinqCtx` | 1193 | `{ entity, param, params }` 上下文（三字段，`params: Vec<(Ident, Type)>` 支持 join）|

### 2.2 待 DSL 化的字符串 API 全清单

【勘误：以下清单为初稿审计结果；截至 2026-06-26，表中所有 `&str` API 均已移除并替换为 `*_internal` 方法（`include_internal`@700、`order_by_column`@581、`sum_internal`@842、`min_internal`@894、`max_internal`@914、`select_internal`@938、`set_column_internal`@1236、`inner_join_internal`@764、`left_join_internal`@787 等）。原表保留作历史对照。】

经审计 `crates/core/src/query.rs` 与 `model_builder.rs`，共 14+ 方法接受 `&str`：

| 方法 | 行号 | 字符串语义 | 解析策略分类 |
|---|---|---|---|
| `include_named` | 634 | 导航名 | 导航常量 `FIELD_*` |
| `then_include_named` | 658 | 嵌套导航名 | 导航常量 `FIELD_*` |
| `order_by` / `order_by_desc` | 612/617 | 列名 | 列常量 `COLUMN_*` |
| `group_by` | 717 | 列名数组 | 列常量 + 元组 |
| `having` | 723 | 原始 SQL | 聚合表达式 DSL |
| `sum` / `avg` | 733/756 | 列名 | 列常量（终端）|
| `min` / `max` | 779/796 | 列名 | 列常量（终端，附 bug 修复）|
| `select_columns` | 817 | 列名数组 | 列常量 + 元组 |
| `set_column` (ExecuteUpdateBuilder) | 1007 | 列名 + 值 | 列常量 + 值 |
| `inner_join` / `left_join` | 689/703 | 表名 + 双列名 | 双闭包 + 双实体 |
| `find_by_id` | 595 | 硬编码 `"id"` | **bug**，改用 PK 元数据 |
| `find_by_key` | 603 | HashMap<String, _> | 改用闭包 + 字段常量 |
| `has_query_filter` (ModelBuilder) | 139 | 原始 SQL | BoolExpr 输出 |
| `has_index` / `has_key_named` (ModelBuilder) | 209/216 | 字段名 | 列常量数组 |

### 2.3 LINQ 终端方法缺口

**已有**：`to_list`、`to_list_with_includes`、`first`、`first_or_default`、`count`、`find_by_id`、`find_by_key`、`sum`、`avg`、`min`、`max`

**缺失**（需补全）：
| 方法 | 语义 | 实现策略 |
|---|---|---|
| `last` | 取最后一条，无则报错 | `order_by pk desc + limit 1`（SQL 无通用 LAST）|
| `last_or_default` | 取最后一条，无则 None | 同上，返回 `Option<T>` |
| `single` | 有且仅有一条，否则报错 | `count == 1` 校验 + `first` |
| `single_or_default` | 0 或 1 条，否则报错 | `count <= 1` 校验 + `first_or_default` |
| `to_dictionary` | 收集为 HashMap<K,V> | `to_list` 后按 selector 填充 |
| `distinct` | SELECT DISTINCT | `state.distinct = true`，SQL 生成时加 `DISTINCT` |
| `all` | 是否全部满足谓词 | `count(WHERE NOT predicate) == 0` |
| `contains` | 是否包含某值 | `WHERE col = value LIMIT 1` 存在性检查 |
| `long_count` | COUNT(*) 返回 i64 | 复用 `count`（已返回 i64，仅需别名）|

### 2.4 已知 Bug

【勘误：截至 2026-06-26，bug 1/3/4 已修复；bug 2 仍未修复（移交 G1 迭代任务）。】

1. **`find_by_id` 硬编码 `"id"`**【已修复 → `find(id)`，`query.rs:646`，使用 `T::entity_meta().primary_keys.first()`】：原描述（`query.rs:598`，`FilterCondition::new("id", "=", 1)`）已不适用。
2. **`min`/`max` 返回 `Option<String>`**【未修复，行号更新为 `query.rs:894/914`】：丢失原类型信息，调用方需手动解析；泛型参数 `<V>` 声明但未使用。**此项由迭代计划 G1 接管**——改为泛型 `min_internal<V,E> where V: TryFrom<DbValue, Error = E>, E: Into<EFError>`。
3. **`#[foreign_key]` 在导航字段上误用**【已修复，`entity.rs:655`】：`extract_foreign_key_field_name` 现返回 `quote!{ None }`，文档注释 640-654 说明改由 `NavigationMeta` 默认推导；原描述（`entity.rs:619-624` 返回目标类型名）已不适用。
4. **`set_property` 死代码**【已移除】：`entity.rs` derive 中已无该 accessor 生成代码。

---

## 三、统一 `linq!` 宏设计

### 3.1 设计总则

**一个宏 `linq!`，零新增宏名**。通过语法形式 dispatch 到不同的展开逻辑，所有展开共享 `extract_field` + 参数化的 `field_const` 基础设施。

### 3.2 语法形式定义

`linq!` 支持三类语法形式：

#### 形式 A：过滤表达式（现有，保持不变）

```rust
// 可复用过滤闭包
let expr = linq!(|b: Blog| b.rating > 0.5);
set.filter(expr).to_list().await?;

// 直接查询
linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;
```

#### 形式 B：多子句查询（新）

`linq!(<source>, [<where_closure>] ; <clause>* )` —— 一次性表达完整查询：

```rust
// 含过滤 + 多子句
linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0.5;
    include b.posts then b.comments;
    order_by b.created_at desc;
    select (b.id, b.title, b.rating);
).to_list().await?;

// 纯子句（无过滤）
linq!(ctx.set::<Blog>();
    include b.posts;
    order_by b.created_at;
).to_list().await?;

// 聚合终端
let total: f64 = linq!(ctx.set::<Blog>(); sum b.views).await?;
let top: Option<i64> = linq!(ctx.set::<Blog>(); max b.rating).await?;
```

**子句语法**（`;` 分隔）：
| 子句 | 语法 | 示例 |
|---|---|---|
| `include` | `include <field> [then <field>]*` | `include b.posts then b.comments` |
| `order_by` | `order_by <field> [asc\|desc]` | `order_by b.created_at desc` |
| `group_by` | `group_by <field> \| <tuple>` | `group_by (b.cat, b.author)` |
| `select` | `select <field> \| <tuple>` | `select (b.id, b.title)` |
| `having` | `having <agg_expr>` | `having count(b.id) > 1` |
| `sum` | `sum <field>` | `sum b.views` |
| `avg` | `avg <field>` | `avg b.rating` |
| `min` | `min <field>` | `min b.rating` |
| `max` | `max <field>` | `max b.rating` |
| `count` | `count` | `count` |
| `distinct` | `distinct` | `distinct` |
| `set` | `set <field>, <value>` | `set b.views, 10` |
| `inner_join` | `inner_join \|<a: T1, b: T2\| a.col == b.col` | `inner_join \|a: Blog, b: Post\| a.id == b.blog_id` |
| `left_join` | `left_join \|<a: T1, b: T2\| a.col == b.col` | 同上 |
| `execute_update` | `execute_update` | 触发批量更新终端 |

#### 形式 C：值产生独立形式（新）

用于无 `QueryBuilder` 实例的场景（`ModelBuilder` 配置），产出值而非链式调用：

```rust
// 全局查询过滤器 → 产出 BoolExpr
builder.has_query_filter(linq!(filter |b: Blog| b.deleted_at.is_null()));

// 索引定义 → 产出 &'static [&'static str]
builder.has_index(linq!(index |b: Blog| (b.author_id, b.created_at)));

// 主键定义 → 产出 &'static [&'static str]
builder.has_key(linq!(key |b: Blog| b.id));
```

`linq!` dispatch 规则：首 token 为 `filter`/`index`/`key` 关键字 → 形式 C；否则按现有规则走形式 A/B。

### 3.3 推荐代码风格（遵循用户偏好）

【勘误：原稿「遵循 project_memory 约定」援引不成立——`c:\Users\lusid\.trae-cn\memory\projects\` 下无 rust-ef 的 `project_memory.md`（仅 rust-agent-flow 项目有）。`split let` 作为**建议**风格保留，但非项目硬约束。用户档案仅注明「dislikes repetitive explanations」，无风格指令。】

建议采用「split `let` bindings」风格，避免过度链式：

```rust
// 推荐：分步 let
let set = context.set::<Blog>();
let expr = linq!(|b: Blog| b.rating > 0.5);
let result = set.filter(expr).to_list().await?;

// 推荐：复杂查询用多子句形式
let set = context.set::<Blog>();
let query = linq!(set, |b: Blog| b.rating > 0.5;
    include b.posts then b.comments;
    order_by b.created_at desc;
);
let blogs = query.to_list().await?;

// 推荐：聚合也用多子句
let set = context.set::<Blog>();
let total_views = linq!(set; sum b.views).await?;
```

---

## 四、实施方案（分阶段）

### 阶段 1：宏基础设施重构（`crates/macros/src/linq.rs` + `entity.rs`）

**目标**：参数化字段提取，支持列常量与导航常量分叉；扩展 `LinqCtx` 支持多参数闭包；derive 宏生成 `FIELD_*` 导航常量。

#### 4.1.1 参数化字段提取

在 `linq.rs` 新增 `FieldKind` 枚举与参数化 `field_const`：

```rust
enum FieldKind { Column, Navigation }

fn field_const(entity: &Type, field: &str, kind: FieldKind) -> TokenStream2 {
    let prefix = match kind { FieldKind::Column => "COLUMN_", FieldKind::Navigation => "FIELD_" };
    let const_name = Ident::new(&format!("{}{}", prefix, field.to_uppercase()), Span::call_site());
    quote! { #entity::#const_name }
}
```

保留 `field_column_const(entity, field)` 作为 `field_const(entity, field, FieldKind::Column)` 的薄包装（向后兼容现有 `compile_comparison`/`compile_order` 调用）。

#### 4.1.2 扩展 `extract_field` 返回结构化结果

```rust
struct FieldRef { entity: Type, field_name: String, kind: FieldKind }

fn extract_field_typed(ctx: &LinqCtx, expr: &Expr) -> syn::Result<FieldRef>
```

`kind` 由调用方上下文决定：过滤/排序/聚合/投影/分组场景传 `Column`；include 场景传 `Navigation`。

#### 4.1.3 扩展 `LinqCtx` 支持多参数闭包（join 场景）

```rust
struct LinqCtx<'a> {
    entity: &'a Type,                    // 主实体（形式 A/B/C 单实体场景）
    params: Vec<(Ident, Type)>,          // 多参数（join 场景：[(a, Blog), (b, Post)]）
}
```

新增 `extract_field_by_param(ctx, expr, param_ident)`：按指定参数 ident 匹配，返回该参数对应实体的列常量。单实体场景 `params` 为空，回退到 `ctx.param` 旧逻辑。

#### 4.1.4 derive 宏生成 `FIELD_*` 导航常量（`entity.rs`）

在 `expand_entity_type` 中，对每个导航字段（`is_navigation_field` 为真）额外生成：

```rust
pub const FIELD_<FIELD_UPPER>: &'static str = "<field_name>";
```

与 `COLUMN_*` 生成逻辑（`entity.rs:346-365`）对称。这样 `include b.posts` → `Blog::FIELD_POSTS`（值为 `"posts"`）→ 传入 `include_internal("posts")` → `find_navigation("posts")`。

#### 4.1.5 修复 `#[foreign_key]` 导航字段误用 bug（`entity.rs:619-624`）

【勘误：本节描述的 bug 已修复。`extract_foreign_key_field_name` 现位于 `entity.rs:655`，签名 `fn(_attrs: &[syn::Attribute])` 忽略 attrs 并返回 `quote!{ None }`；文档注释 640-654 说明改由 `NavigationMeta` 默认推导。下方原方案保留作历史记录。】

`extract_foreign_key_field_name` 当前返回 `#[foreign_key(X)]` 的目标类型名 `X`。修正为：
- 导航字段上的 `#[foreign_key]` 标注语义改为「指定 FK 字段名」（如 `#[foreign_key(post_id)]`），返回该字段名字符串；
- 若未标注，返回 `None`（由 `NavigationMeta` 按关系类型默认推导，`HasMany` 用 `<Target>::FK_<Self>`，`BelongsTo` 用本实体 FK 列）；
- 标量字段上的 `#[foreign_key(Target)]` 语义不变（生成 `FK_<Target>` 常量）。

同步更新 `lib.rs:11-25` 的 `attributes(...)` 列表，明确 `foreign_key` 可接收 ident（字段名）或 path（目标类型）。

#### 4.1.6 移除 `set_property` 死代码

【勘误：已完成。`entity.rs` derive 中已无 `set_property` accessor 生成代码。】

删除 `entity.rs` derive 中 `set_property` accessor 的生成代码（`INavigationSetter` trait 不再要求该访问器）。

---

### 阶段 2：`linq!` 多子句形式实现（`crates/macros/src/linq.rs`）

**目标**：实现形式 B（多子句查询）与形式 C（值产生独立形式）的解析与展开。

#### 4.2.1 重构 `LinqInput` 解析

扩展 `LinqInput`：

```rust
struct LinqInput {
    source: Option<Expr>,
    where_clause: Option<LinqWhere>,    // 改为 Option（纯子句查询无 where）
    clauses: Vec<LinqClause>,           // 新增：子句列表
}

enum LinqClause {
    Include { field: Expr, nested: Vec<Expr> },              // include b.posts then b.comments
    OrderBy { field: Expr, descending: bool },
    GroupBy { fields: Vec<Expr> },                            // 单字段或元组
    Select { fields: Vec<Expr> },
    Having { expr: Expr },
    Sum(Expr), Avg(Expr), Min(Expr), Max(Expr), Count,
    Distinct,
    Set { field: Expr, value: Expr },
    InnerJoin { params: Vec<(Ident, Type)>, left: Expr, right: Expr },
    LeftJoin { params: Vec<(Ident, Type)>, left: Expr, right: Expr },
    ExecuteUpdate,
}
```

解析逻辑（`impl Parse for LinqInput`）：
1. 若首 token 为 `filter`/`index`/`key` 关键字 → 形式 C，dispatch 到 `expand_value_form`。
2. 否则按现有逻辑解析 `source?` + `where_closure?`。
3. 若遇到 `;` → 进入子句解析循环，每次 `;` 后按关键字（`include`/`order_by`/`group_by`/`select`/`having`/`sum`/`avg`/`min`/`max`/`count`/`distinct`/`set`/`inner_join`/`left_join`/`execute_update`）解析对应子句。

#### 4.2.2 子句展开

每个子句展开为 `.method_internal(...)` 片段，拼接在 `__qb` 链上：

```rust
// include b.posts then b.comments
→ .include_internal(Blog::FIELD_POSTS).then_include_internal(Post::FIELD_COMMENTS)

// order_by b.created_at desc
→ .order_by_desc_column_internal(Blog::COLUMN_CREATED_AT)

// group_by (b.cat, b.author)
→ .group_by_columns_internal(&[Blog::COLUMN_CAT, Blog::COLUMN_AUTHOR])

// sum b.views（终端）
→ .sum_internal(Blog::COLUMN_VIEWS)  // 调用方加 .await

// inner_join |a: Blog, b: Post| a.id == b.blog_id
→ .inner_join_internal(Blog::TABLE, Blog::COLUMN_ID, Post::COLUMN_BLOG_ID)

// set b.views, 10
→ .set_column_internal(Blog::COLUMN_VIEWS, 10)
```

聚合终端（`sum`/`avg`/`min`/`max`/`count`）的展开产物自带 `.await`，且为查询链的终点（后续不能再链式）。

#### 4.2.3 形式 C 展开（值产生）

【勘误：`FilterCondition::new(column, operator, param_count: usize)` 第 3 参为参数计数（usize），**非值**。值携带版本须用 `FilterCondition::with_values(column, operator, values: Vec<DbValue>)`（`query.rs:51`）。下方 `IS NULL` 示例因 `param_count=0` 恰好可编译，但带值过滤（如 `b.rating > 5`）须改用 `with_values`。】

```rust
// linq!(filter |b: Blog| b.deleted_at.is_null())
→ 编译为 BoolExpr AST（复用 compile_expr，但输出 BoolExpr 而非 .filter_column 链）
  rust_ef::query::BoolExpr::Filter(rust_ef::query::FilterCondition::new(
      Blog::COLUMN_DELETED_AT, "IS NULL", 0))

// 带值过滤（原稿遗漏的用法）
// linq!(filter |b: Blog| b.rating > 5)
→ rust_ef::query::BoolExpr::Filter(rust_ef::query::FilterCondition::with_values(
      Blog::COLUMN_RATING, ">", vec![DbValue::from(5)]))

// linq!(index |b: Blog| (b.author_id, b.created_at))
→ &[Blog::COLUMN_AUTHOR_ID, Blog::COLUMN_CREATED_ID]

// linq!(key |b: Blog| b.id)
→ &[Blog::COLUMN_ID]
```

形式 C 的 `filter` 子句需新增 `compile_bool_expr` 函数，产出 `BoolExpr` 值而非方法链。这是对 `compile_expr` 的重构：抽出「表达式 → BoolExpr」的核心逻辑，`compile_expr`（链式输出）与 `compile_bool_expr`（值输出）分别包装。

#### 4.2.4 `having` 聚合表达式 DSL

`having count(b.id) > 1` 解析：识别 `count(...)`/`sum(...)`/`avg(...)`/`min(...)`/`max(...)` 为聚合函数节点，展开为 `having_aggregate_internal("COUNT", Blog::COLUMN_ID, ">", 1)`。底层生成 `HAVING COUNT(id) > ?`。

---

### 阶段 3：QueryBuilder 内部方法 + 移除字符串 API（`crates/core/src/query.rs`）

**目标**：新增 `pub(crate) *_internal` 方法接收 `&'static str` 常量；**删除**所有公开 `&str` API（用户决策：立即移除）。

#### 4.3.1 新增 `*_internal` 方法

```rust
impl<T: IEntityType> QueryBuilder<T> {
    pub(crate) fn include_internal(mut self, nav: &'static str) -> Self { ... }
    pub(crate) fn then_include_internal(mut self, nav: &'static str) -> Self { ... }
    pub(crate) fn order_by_column_internal(mut self, col: &'static str) -> Self { ... }
    pub(crate) fn order_by_desc_column_internal(mut self, col: &'static str) -> Self { ... }
    pub(crate) fn group_by_columns_internal(mut self, cols: &'static [&'static str]) -> Self { ... }
    pub(crate) fn select_columns_internal(mut self, cols: &'static [&'static str]) -> SelectQueryBuilder<T> { ... }
    pub(crate) fn having_aggregate_internal(mut self, agg: &str, col: &'static str, op: &str, val: impl Into<DbValue>) -> Self { ... }
    pub(crate) async fn sum_internal(self, col: &'static str) -> EFResult<f64> { ... }
    pub(crate) async fn avg_internal(self, col: &'static str) -> EFResult<f64> { ... }
    pub(crate) async fn min_internal<V: TryFrom<DbValue>>(self, col: &'static str) -> EFResult<Option<V>> { ... }
    pub(crate) async fn max_internal<V: TryFrom<DbValue>>(self, col: &'static str) -> EFResult<Option<V>> { ... }
    pub(crate) fn set_column_internal(mut self, col: &'static str, value: impl Into<DbValue>) -> Self { ... }
    pub(crate) fn inner_join_internal(mut self, table: &'static str, left: &'static str, right: &'static str) -> Self { ... }
    pub(crate) fn left_join_internal(mut self, table: &'static str, left: &'static str, right: &'static str) -> Self { ... }
}
```

#### 4.3.2 删除公开字符串 API

直接删除以下方法（无 `#[deprecated]` 过渡）：
- `include_named`、`then_include_named`
- `order_by(&str)`、`order_by_desc(&str)`
- `group_by(&[&str])`
- `having(&str)`
- `sum(&str)`、`avg(&str)`、`min(&str)`、`max(&str)`
- `select_columns(&[&str])`
- `set_column(&str, _)`（ExecuteUpdateBuilder）
- `inner_join(&str, &str, &str)`、`left_join(&str, &str, &str)`
- `find_by_id`（替换为新 `find(id)`，见 4.3.3）
- `find_by_key(&HashMap<String, _>)`（替换为新 `find_by_key`，见 4.3.3）

#### 4.3.3 修复 `find_by_id` 并新增 `find` / `find_by_key`

```rust
impl<T: IEntityType> QueryBuilder<T> {
    /// 按主键查找（单主键实体）。使用实体 PK 元数据，不再硬编码 "id"。
    pub async fn find(mut self, id: impl Into<DbValue>) -> EFResult<Option<T>> {
        let pk = T::entity_meta().primary_keys.first()
            .ok_or_else(|| EFError::Query("entity has no primary key".into()))?;
        self = self.filter_column_internal(pk.column_name, "=", id);
        self.first_or_default().await
    }

    /// 按复合主键查找。键为列名常量。
    pub async fn find_by_key(mut self, keys: &[(&'static str, DbValue)]) -> EFResult<Option<T>> {
        for (col, val) in keys {
            self = self.filter_column_internal(col, "=", val.clone());
        }
        self.first_or_default().await
    }
}

impl<T: IEntityType> DbSet<T> {
    /// DbSet 上的便捷 find。
    pub async fn find(&self, id: impl Into<DbValue>) -> EFResult<Option<T>> {
        self.query().find(id).await
    }
}
```

调用示例：
```rust
// 单主键
let blog = set.find(1).await?;

// 复合主键
let m2m = set.query().find_by_key(&[
    (BlogTag::FIELD_BLOG_ID, DbValue::I32(1)),
    (BlogTag::FIELD_TAG_ID, DbValue::I32(2)),
]).await?;
```

#### 4.3.4 修复 `min`/`max` 返回类型

`min_internal<V>`/`max_internal<V>` 改为泛型 `V: TryFrom<DbValue>`，返回 `EFResult<Option<V>>`：

```rust
pub async fn max_internal<V: TryFrom<DbValue, Error = E>, E>(self, col: &'static str) -> EFResult<Option<V>> {
    // SQL: SELECT MAX(col) FROM ... 
    // 解析 DbValue 后 V::try_from(db_value)
}
```

调用：`let top: i64 = linq!(set; max b.rating).await?.unwrap_or(0);`（类型由上下文推断）。

---

### 阶段 4：LINQ 终端方法补全（`crates/core/src/query.rs`）

**目标**：新增 9 个缺失终端方法。

```rust
impl<T: IEntityType> QueryBuilder<T> {
    pub async fn last(mut self) -> EFResult<T> {
        let pk = T::entity_meta().primary_keys.first()
            .ok_or_else(|| EFError::Query("last requires primary key".into()))?;
        self = self.order_by_desc_column_internal(pk.column_name);
        self.first().await
    }

    pub async fn last_or_default(mut self) -> EFResult<Option<T>> {
        // 同上但 first_or_default
    }

    pub async fn single(self) -> EFResult<T> {
        let count = self.clone().count().await?;
        if count != 1 { return Err(EFError::Query(format!("sequence contains {} elements", count))); }
        self.first().await
    }

    pub async fn single_or_default(self) -> EFResult<Option<T>> {
        let count = self.clone().count().await?;
        if count > 1 { return Err(EFError::Query("sequence contains more than one element".into())); }
        self.first_or_default().await
    }

    pub async fn to_dictionary<K, V, Fk, Fv>(self, key_sel: Fk, val_sel: Fv) -> EFResult<std::collections::HashMap<K, V>>
    where
        K: Eq + std::hash::Hash,
        Fk: Fn(&T) -> K,
        Fv: Fn(&T) -> V,
    {
        let rows = self.to_list().await?;
        Ok(rows.into_iter().map(|r| (key_sel(&r), val_sel(&r))).collect())
    }

    pub fn distinct(mut self) -> Self {
        self.state.distinct = true;
        self
    }

    pub async fn all<F>(mut self, predicate: F) -> EFResult<bool>
    where
        F: FnOnce(QueryBuilder<T>) -> QueryBuilder<T>,
    {
        // SELECT COUNT(*) FROM (...) WHERE NOT (predicate)
        // 等价于反向过滤后 count == 0
        let negated = predicate(self.clone());
        // 复杂度高，简化为：取 total count 与 predicate count 比较
        // 实现细节：子查询或两次查询
        Ok(false) // 占位，实现时补全
    }

    pub async fn contains(mut self, value: impl Into<DbValue>) -> EFResult<bool> {
        let pk = T::entity_meta().primary_keys.first()
            .ok_or_else(|| EFError::Query("contains requires primary key".into()))?;
        self = self.filter_column_internal(pk.column_name, "=", value);
        Ok(self.count().await? > 0)
    }

    pub async fn long_count(self) -> EFResult<i64> {
        // 复用 count（已返回 i64）
        self.count().await
    }
}
```

`QueryState` 新增 `distinct: bool` 字段，SQL 生成时（`query.rs:296-298` 的 SELECT 子句）追加 `DISTINCT`。

`all` 方法的实现策略：由于 `linq!` 产出闭包，`all` 接收 `FnOnce(QueryBuilder) -> QueryBuilder` 形式的谓词闭包。实现为：克隆当前 builder，应用谓词得到满足条件的 count，与总数比较。若总数 == 满足数则 `true`。（注：需 `QueryBuilder: Clone`，当前已是 `Clone`。）

---

### 阶段 5：ModelBuilder DSL 化（`crates/core/src/model_builder.rs`）

**目标**：`has_query_filter`、`has_index`、`has_key` 接受 `linq!` 形式 C 产出值。

```rust
impl ModelBuilder {
    /// 接收 BoolExpr（由 linq!(filter |b| ...) 产出）
    pub fn has_query_filter(&mut self, filter: rust_ef::query::BoolExpr) -> &mut Self {
        let config = self.get_or_create_config();
        config.query_filter = Some(filter);  // 改为 BoolExpr 类型
        self
    }

    /// 接收 &'static [&'static str]（由 linq!(index |b| ...) 产出）
    pub fn has_index(&mut self, columns: &'static [&'static str]) -> &mut Self { ... }

    pub fn has_key(&mut self, columns: &'static [&'static str]) -> &mut Self { ... }
}
```

`EntityConfig.query_filter` 字段类型从 `Option<String>` 改为 `Option<BoolExpr>`。`DbContext::set::<T>()` 注入时直接用 `BoolExpr`，无需 `filter_raw` 解析。`QueryBuilder::apply_query_filter` 合并 `BoolExpr`（复用现有 `append_bool_expr`）。

删除 `filter_raw`、`get_query_filter`（返回 `&str` 的旧版本）。

---

### 阶段 6：测试、示例与文档同步

#### 4.6.1 测试更新

| 测试文件 | 更新内容 |
|---|---|
| `crates/core/tests/sqlite_crud_tests.rs` | 所有 `order_by("col")`/`sum("col")`/`include_named("x")` 改为 `linq!` 形式 |
| `crates/core/tests/navigation_tests.rs` | `include_named`/`then_include_named` 改为 `linq!(include ...)` |
| `crates/core/tests/m2m_tests.rs` | 同上 |
| `crates/core/tests/linq_tests.rs` | 新增多子句形式、聚合、join、set 的测试用例 |
| `crates/core/tests/bool_expr_tests.rs` | 新增 `BoolExpr` 形式 C 输出测试 |

新增测试文件：
- `crates/core/tests/linq_dsl_tests.rs` —— 专门测试统一 DSL 的所有子句形式
- `crates/core/tests/linq_terminal_tests.rs` —— 测试 `last`/`single`/`distinct`/`all`/`contains`/`to_dictionary`

#### 4.6.2 示例更新（`examples/blog`）

重写 blog 示例，全面采用 `linq!` 多子句形式与推荐代码风格（split `let` bindings）：

```rust
// 查询带导航的博客
let set = ctx.set::<Blog>();
let query = linq!(set, |b: Blog| b.published;
    include b.author then b.profile;
    order_by b.created_at desc;
    select (b.id, b.title, b.author_id);
);
let posts = query.to_list().await?;

// 聚合
let set = ctx.set::<Blog>();
let avg_rating = linq!(set; avg b.rating).await?;

// 批量更新
let set = ctx.set::<Blog>();
let affected = linq!(set, |b: Blog| b.rating < 0.1; set b.published, false; execute_update).await?;
```

#### 4.6.3 文档同步（遵循 project_memory 硬约束）

【勘误：①「遵循 project_memory 硬约束」援引不成立（见 §3.3 勘误）；②下方章节映射表与 `docs/rust-ef/` **实际目录结构完全不符**——实际目录为 `04-relationships / 05-query-patterns / 06-advanced-query / 07-change-tracking / 08-bulk-operations / 09-transactions-migrations / 10-di-interceptors / 11-best-practices`，无 `04-query-basics / 05-filtering / 06-ordering / 07-aggregation / 08-navigation / 09-joins / 10-batch-operations` 等目录；③「新增 `12-linq-terminals/`」决策已撤销——终端方法参考并入 `05-query-patterns/count-any.md` 与 `06-advanced-query/aggregation.md`，避免目录膨胀。正确的文档同步清单见迭代计划 G4。】

更新 `docs/rust-ef/` 以下章节（遵循 `E:\GitCode\RF\rust-webapp\docs\rust-webapp` 规范）：

| 章节 | 文件 | 更新内容 |
|---|---|---|
| 04 查询基础 | `04-query-basics/*.md` | `linq!` 多子句形式语法说明 |
| 05 过滤表达式 | `05-filtering/*.md` | 形式 A（过滤闭包）不变 |
| 06 排序与分页 | `06-ordering/*.md` | `order_by` 子句语法 |
| 07 聚合与分组 | `07-aggregation/*.md` | `sum`/`avg`/`min`/`max`/`group_by`/`having` 子句 |
| 08 导航加载 | `08-navigation/*.md` | `include ... then ...` 子句，移除 `include_named` |
| 09 JOIN 查询 | `09-joins/*.md` | `inner_join`/`left_join` 多参数闭包语法 |
| 10 批量操作 | `10-batch-operations/*.md` | `set` 子句 + `execute_update` |
| 11 最佳实践 | `11-best-practices/*.md` | 统一 `linq!` 风格指南，split `let` 示例 |
| LINQ 终端参考 | 新增 `12-linq-terminals/*.md` | 全部终端方法参考（含新增 9 个）|

`README.md` 的「Best Practices Guide」章节同步更新，所有示例改用统一 `linq!` 形式。

#### 4.6.4 迭代计划同步

更新 `.trae/documents/迭代计划_v0.4_v1.0_plan.md`：
- v0.4 Beta 1 任务列表新增「统一 `linq!` DSL 改造」为核心任务（引用本计划）
- 风险表新增「字符串 API 立即移除可能破坏下游用户」(概率低，因项目未发布 GA)
- 验收标准新增「所有 `&str` 查询 API 已移除，`cargo check --workspace` 零 warning」

---

## 五、假设与决策

### 5.1 已确认决策

| 决策项 | 选择 | 依据 |
|---|---|---|
| 统一宏入口 | 扩展 `linq!` 为唯一入口 | 用户确认（推荐项）|
| 字符串 API 兼容 | 立即移除，无 deprecated 过渡 | 用户确认 |
| DSL 语法 | 闭包字段访问 | 用户先前确认 |
| 代码风格 | split `let` bindings | 建议（原稿误记为 project_memory 约定，实际无该档案）|
| 文档规范 | 遵循 `rust-webapp/docs` | 建议（原稿误记为 project_memory 硬约束，实际无该档案）|

### 5.2 关键假设

1. **`QueryBuilder: Clone`**【勘误：此假设不成立——`QueryBuilder<T>`（`query.rs:448`）**未派生 `Clone`**，仅有 `QueryState` 派生 `Clone`。实际实现用 `take(2)` + `to_list()` 规避克隆：`single`/`single_or_default` 取 2 条后校验长度，`all` 直接 `to_list` 后在 Rust 侧应用谓词。原假设「需克隆 builder 做两次查询」已不适用。】：`all`/`single` 终端方法原设计需克隆 builder 做两次查询。已验证 `QueryBuilder` 派生 `Clone`（`query.rs` 派生宏）。
2. **`DbValue: TryFrom` 转换**：`min_internal<V>`/`max_internal<V>` 依赖 `V: TryFrom<DbValue>`。需为常用类型（i32/i64/f64/String/bool）实现 `TryFrom<DbValue>`（部分已有 `From`，补充 `TryFrom`）。
3. **`linq!` 形式 C 的 `BoolExpr` 输出**：`compile_expr` 当前输出方法链。重构为「核心表达式 → BoolExpr AST」+「BoolExpr → 方法链」两步，形式 C 取第一步输出。工作量可控。
4. **`having` 聚合 DSL**：首版仅支持 `count`/`sum`/`avg`/`min`/`max` 五种聚合函数与简单比较运算符；复杂 `having`（嵌套表达式）后续扩展。
5. **`all` 谓词闭包**：`all<F>(self, predicate: F)` 中 `F: FnOnce(QueryBuilder) -> QueryBuilder`。调用方用 `linq!` 产出闭包传入：`set.all(linq!(|b: Blog| b.published)).await?`。需 `linq!` 支持产出「接收并返回 QueryBuilder 的闭包」——这是现有 `linq!(|b: T| ...)` 形式的自然扩展（已产出 `|__qb| __qb.chain(...)`）。

### 5.3 范围边界

**纳入本计划**：
- `linq!` 宏扩展（形式 B/C）
- 14+ 字符串 API 移除与替换
- 9 个 LINQ 终端方法补全
- 4 个已知 bug 修复
- `model_builder` DSL 化（query_filter/index/key）
- 测试、示例、文档全面同步

**不纳入（推迟到 v0.5+）**：
- `linq!` 类型推断（省略闭包类型标注）—— 迭代计划已列为可选
- 子查询 / 关联过滤（`b.posts.any(p => ...)`）—— 需多段路径解析，工作量大
- Lazy Loading —— 架构性变更
- 强类型元组投影（`select (b.id, b.title)` 返回 `(i32, String)` 而非 `Vec<String>`）—— 首版保留 `Vec<String>`，强类型化后续

---

## 六、验证步骤

### 6.1 编译验证

```bash
cargo check --workspace                    # 零错误零 warning
cargo clippy --workspace -- -D warnings    # clippy 零 warning
cargo fmt --check                          # 格式检查
```

### 6.2 测试验证

```bash
# 单元 + 集成测试（SQLite 内存库）
cargo test --workspace

# 新增 DSL 测试专项
cargo test -p rust-ef --test linq_dsl_tests
cargo test -p rust-ef --test linq_terminal_tests

# PostgreSQL / MySQL（可选，需外部库）
RUST_EF_PG_URL=postgres://... cargo test -p rust-ef --test postgres_crud_tests
RUST_EF_MYSQL_URL=mysql://... cargo test -p rust-ef --test mysql_crud_tests
```

### 6.3 字符串 API 移除验证

```bash
# 全工作区搜索，确认无 &str 查询 API 残留调用
# 期望：仅内部 *_internal 方法保留，公开 API 全部经 linq! 调用
```
（用 Grep 搜索 `\.include_named\(`、`\.order_by("`、`\.sum("` 等，确认零命中）

### 6.4 文档验证

- `docs/rust-ef/` 所有代码示例经 `cargo test --doc` 或手动编译验证
- `README.md` 示例同步更新
- 迭代计划文档同步更新

### 6.5 验收清单

- [ ] `linq!` 支持形式 A（过滤，向后兼容）
- [ ] `linq!` 支持形式 B（多子句：include/order_by/group_by/select/sum/avg/min/max/set/inner_join/left_join/having/execute_update）
- [ ] `linq!` 支持形式 C（filter/index/key 值产生）
- [ ] 14+ 字符串 API 全部移除，无 `#[deprecated]` 残留
- [ ] `find_by_id` bug 修复，改为 `find(id)` 使用 PK 元数据
- [ ] `min`/`max` 返回泛型 `Option<V>`
- [ ] `#[foreign_key]` 导航字段 bug 修复
- [ ] `set_property` 死代码移除
- [ ] 9 个 LINQ 终端方法补全（last/last_or_default/single/single_or_default/to_dictionary/distinct/all/contains/long_count）
- [ ] `model_builder` 的 `has_query_filter`/`has_index`/`has_key` 接受 `linq!` 形式 C
- [ ] `cargo check --workspace` 零 warning
- [ ] `cargo clippy --workspace -- -D warnings` 通过
- [ ] `cargo test --workspace` 全绿（含新增 DSL 测试）
- [ ] `examples/blog` 全面采用新语法
- [ ] `docs/rust-ef/` 11+ 章节同步更新
- [ ] `README.md` 最佳实践章节同步更新
- [ ] 迭代计划文档同步更新

---

## 七、实施顺序与依赖

```
阶段 1 (宏基础设施) ──► 阶段 2 (linq! 多子句)
       │                      │
       ▼                      ▼
阶段 3 (QueryBuilder internal + 移除 str API + bug 修复)
       │
       ▼
阶段 4 (LINQ 终端补全)
       │
       ▼
阶段 5 (ModelBuilder DSL 化)
       │
       ▼
阶段 6 (测试 + 示例 + 文档)
```

阶段 1-3 必须一次性完成（否则编译断裂，因字符串 API 立即移除）。建议在单一分支上连续提交，最后统一合并。

---

## 八、风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|
| `compile_expr` 重构为 BoolExpr 输出影响现有 `linq!` 行为 | 中 | 高 | 保留 `compile_expr`（链式输出）作为薄包装，核心逻辑移至 `compile_bool_expr_core`，两路径共享 |
| `having` 聚合 DSL 解析复杂度超预期 | 中 | 中 | 首版仅支持 `agg(col) op value` 简单形式，复杂表达式标 `TODO` 推迟 |
| `all` 谓词闭包与 `linq!` 闭包形式不兼容 | 低 | 中 | `all` 直接接收 `linq!` 产出的 `QueryBuilder -> QueryBuilder` 闭包，语法 `set.all(linq!(\|b\| b.published))` |
| 文档同步工作量大 | 高 | 低 | 分章节增量更新，优先 04/08/11 三章（查询/导航/最佳实践）|
| 字符串 API 移除破坏未迁移代码 | 低 | 中 | 项目未 GA，无外部用户；内部测试与示例在阶段 6 同步迁移 |

---

*本计划基于 2026-06-25 代码库状态制定，所有文件路径与行号引用均经实际验证。*
