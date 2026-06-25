# rust-ef DSL 类型安全 API 改造计划

> 编制日期: 2026-06-25  
> 目标: 将 `include_named`/`then_include_named`/`order_by`/`group_by`/`sum`/`set_column` 等所有字符串参数 API 替换为闭包 DSL，并补全 LINQ 终端方法  
> 决策: 采用闭包字段访问 DSL（与 `linq!` 风格一致）；废弃字符串 API（标记 `#[deprecated]` + `#[doc(hidden)]`，内部保留 `pub(crate)` 实现）

---

## 一、现状分析

### 1.1 已有基础设施

| 基础设施 | 位置 | 说明 |
|---------|------|------|
| `linq!` 宏 | [crates/macros/src/linq.rs:176-214](file:///E:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L176-L214) | 已实现闭包解析 → 字段名提取 → `#entity::COLUMN_<FIELD>` 常量引用 |
| `extract_field` | [crates/macros/src/linq.rs:440-499](file:///E:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L440-L499) | 从 `b.rating` 提取字段名 "rating" |
| `field_column_const` | [crates/macros/src/linq.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/linq.rs) | 生成 `Blog::COLUMN_RATING` |
| `COLUMN_*` 常量 | derive 宏生成（[entity.rs:355-365](file:///E:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L355-L365)） | 每个普通字段生成 `pub const COLUMN_<NAME>: &str` |
| `column!` 宏 | [crates/macros/src/column_macro.rs:12-48](file:///E:/GitCode/RF/rust-ef/crates/macros/src/column_macro.rs#L12-L48) | `column!(Blog::url)` → `Blog::COLUMN_URL` |
| `find_navigation` | [metadata.rs:214](file:///E:/GitCode/RF/rust-ef/crates/core/src/metadata.rs#L214) | 按字段名查导航元数据 |

### 1.2 字符串 API 全清单（需 DSL 化）

**QueryBuilder（[query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs)）**:

| 方法 | 行号 | 字符串参数 | DSL 目标 |
|------|:----:|-----------|---------|
| `order_by` | 612 | `column: &str` | `order_by!(\|b: Blog\| b.rating)` |
| `order_by_desc` | 617 | `column: &str` | `order_by_desc!(\|b: Blog\| b.rating)` |
| `group_by` | 717 | `columns: &[&str]` | `group_by!(\|b: Blog\| b.category_id)` 或元组多列 |
| `having` | 723 | `expression: &str`（原始 SQL） | `having!(\|b\| b.count > 5)`（表达式树） |
| `sum` | 733 | `column: &str` | `sum!(\|b: Blog\| b.rating)` |
| `avg` | 756 | `column: &str` | `avg!(\|b: Blog\| b.rating)` |
| `min` | 779 | `column: &str` | `min!(\|b: Blog\| b.rating)` |
| `max` | 796 | `column: &str` | `max!(\|b: Blog\| b.rating)` |
| `include_named` | 634 | `navigation: &str` | `include!(\|b: Blog\| b.posts)` |
| `then_include_named` | 658 | `navigation: &str` | `then_include!(\|p: Post\| p.comments)` |
| `inner_join` | 689 | `table, left_col, right_col` | `inner_join!(Blog, \|p: Post\| p.blog_id, \|b: Blog\| b.blog_id)` |
| `left_join` | 703 | `table, left_col, right_col` | `left_join!(...)` |
| `select_columns` | 817 | `columns: &[&str]` | `select!(\|b: Blog\| (b.url, b.rating))` |
| `find_by_id` | 595 | 硬编码 `"id"` | `find(id)` 终端方法，动态取主键列 |
| `find_by_key` | 603 | `HashMap<String, DbValue>` | `find_by_key!({ \|b\| b.id => 1, \|b\| b.tenant => 2 })` |

**ExecuteUpdateBuilder（[query.rs:1007](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1007)）**:
- `set_column(column: &str, value)` → `set!(|b: Blog| b.rating, 5)`

**ModelBuilder（[model_builder.rs:139](file:///E:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs#L139)）**:
- `has_query_filter(filter_sql: &str)` → `query_filter!(|b: Blog| b.deleted_at.is_null())`

**DbSet（[db_set.rs:225](file:///E:/GitCode/RF/rust-ef/crates/core/src/db_set.rs#L225)）**:
- `exists_by_id(HashMap<String, DbValue>)` → 复用 `find` 终端方法

### 1.3 LINQ 终端方法缺口

**已有**: `to_list`、`first`、`first_or_default`、`count`、`any`、`sum`、`avg`、`min`、`max`、`execute_delete`、`execute_update`

**缺失（需补全）**:

| 方法 | EF Core 等价 | 语义 |
|------|-------------|------|
| `last_or_default` | `LastOrDefault` | 反向排序取首条 |
| `single` | `Single` | 恰好一行，否则报错 |
| `single_or_default` | `SingleOrDefault` | 0 或 1 行，多行报错 |
| `to_dictionary` | `ToDictionary` | 按 key 投影为 HashMap |
| `distinct` | `Distinct` | 去重 |
| `all` | `All(predicate)` | 全部满足 |
| `contains` | `Contains(item)` | 包含某实体 |
| `long_count` | `LongCount` | API 名称对齐（现有 count 已返回 i64） |

### 1.4 发现的 Bug 与设计缺陷

| 问题 | 位置 | 严重性 |
|------|------|:------:|
| `find_by_id` 硬编码列名 `"id"` | [query.rs:598](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs#L598) | 高 |
| `find_by_id` 只接受 `i32` | [query.rs:595](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs#L595) | 中 |
| `ExecuteUpdateBuilder::set_property` accessor 未使用（`_accessor`） | [query.rs:1001](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1001) | 高 |
| `min`/`max` 返回 `Option<String>` 而非强类型 | [query.rs:779,796](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs#L779) | 中 |
| `min<V>`/`max<V>` 残留未使用泛型参数 | [query.rs:779,796](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs#L779) | 低 |
| `#[foreign_key]` 把目标类型名填入 `foreign_key_field` | [entity.rs:619-624](file:///E:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L619) | 中 |
| `#[navigation]` 属性注册但未消费 | [lib.rs:19](file:///E:/GitCode/RF/rust-ef/crates/macros/src/lib.rs#L19) | 低 |

---

## 二、设计方案

### 2.1 DSL 宏设计原则

1. **复用 `linq!` 机制**：所有新宏复用 `extract_field` + `field_column_const` 提取列名
2. **闭包参数类型推断**：
   - 单实体上下文宏（`order_by!`/`sum!`/`include!`）可省略类型，从 `QueryBuilder<T>` 推断
   - 跨实体宏（`then_include!`/`inner_join!`）需显式标注或通过额外泛型参数传递
3. **底层 `*_named` 方法保留为 `pub(crate)`**：DSL 宏展开为这些方法的调用，不破坏内部实现
4. **公开 `&str` API 标记 `#[deprecated]` + `#[doc(hidden)]`**：编译期警告引导迁移

### 2.2 宏展开示例

```rust
// include!(|b: Blog| b.posts)
// 展开为:
query.include_named_internal(Blog::FIELD_POSTS)

// then_include!(|p: Post| p.comments)
// 展开为:
query.then_include_named_internal(Post::FIELD_COMMENTS)

// order_by!(|b: Blog| b.rating)
// 展开为:
query.order_by_column_internal(Blog::COLUMN_RATING)

// sum!(|b: Blog| b.rating)
// 展开为:
query.sum_internal(Blog::COLUMN_RATING).await

// set!(|b: Blog| b.rating, 5)
// 展开为:
update.set_column_internal(Blog::COLUMN_RATING, 5)

// group_by!(|b: Blog| (b.category_id, b.author_id))
// 展开为:
query.group_by_columns_internal(&[Blog::COLUMN_CATEGORY_ID, Blog::COLUMN_AUTHOR_ID])
```

### 2.3 导航字段常量生成

derive 宏扩展：为每个导航字段生成 `pub const FIELD_<NAME>: &str = "<name>"` 常量，供 `include!`/`then_include!` 使用。

```rust
// 在 entity.rs:355-365 附近扩展
for nav_field in &navigation_fields {
    let const_name = Ident::new(&format!("FIELD_{}", nav_field.to_uppercase()), span);
    let field_name = &nav_field;
    quote! { pub const #const_name: &'static str = #field_name; }
}
```

### 2.4 then_include 类型推断策略

`then_include!(|p: Post| p.comments)` 的闭包参数类型 `Post` 必须显式标注，因为：
- 链式调用 `.include!(|b: Blog| b.posts).then_include!(|p: Post| p.comments)` 中，宏在展开时无法从上一个 `include!` 推断出 `Post` 类型
- 这与 `linq!` 要求 `|b: Blog|` 标注的约束一致，保持一致性

**未来优化**（不在本计划范围）：研究通过 `IncludeChain<T, Nav>` 类型封装实现链式类型推断，但会显著增加 `QueryBuilder` 复杂度。

---

## 三、实施步骤

### 阶段 1: 导航加载 DSL（include!/then_include!）

**目标**: 替换 `include_named`/`then_include_named`

**改动文件**:
- [crates/macros/src/lib.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/lib.rs) — 注册 `include!`/`then_include!` proc_macro
- [crates/macros/src/include_macro.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/include_macro.rs) — **新建**，实现两个宏
- [crates/macros/src/entity.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/entity.rs) — 为导航字段生成 `FIELD_<NAME>` 常量
- [crates/core/src/query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs) — `include_named`/`then_include_named` 拆分为 `pub(crate) include_named_internal` + `#[deprecated] pub include_named`

**步骤**:
1. 在 `entity.rs` 导航字段处理逻辑中，为每个导航字段生成 `pub const FIELD_<NAME>: &str`
2. 新建 `include_macro.rs`，实现 `include!`/`then_include!`：
   - 解析闭包 `|b: Blog| b.posts`
   - 复用 `extract_field` 提取字段名
   - 展开为 `.include_named_internal(#entity::FIELD_POSTS)`
3. 在 `lib.rs` 注册两个 proc_macro
4. 在 `query.rs` 中：
   - `include_named` 重命名为 `include_named_internal`（`pub(crate)`）
   - 新增 `#[deprecated(note="use include! macro")] #[doc(hidden)] pub fn include_named` 委托 internal
   - `then_include_named` 同理
5. 在 `macros/Cargo.toml` 中将 `include_macro` 模块加入 lib

**验证**:
- 现有 `navigation_tests.rs` 改用 `include!`/`then_include!`，测试全绿
- `include_named` 调用产生 deprecation 警告

### 阶段 2: 排序/分组/聚合 DSL

**目标**: 替换 `order_by`/`order_by_desc`/`group_by`/`sum`/`avg`/`min`/`max`

**改动文件**:
- [crates/macros/src/lib.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/lib.rs) — 注册新宏
- [crates/macros/src/query_dsl_macros.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/query_dsl_macros.rs) — **新建**，统一实现排序/分组/聚合宏
- [crates/core/src/query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs) — 拆分 `*_internal` + deprecated 公开版

**宏清单**:
- `order_by!(|b: Blog| b.rating)` → `.order_by_column_internal(Blog::COLUMN_RATING)`
- `order_by_desc!(|b: Blog| b.rating)` → `.order_by_desc_column_internal(Blog::COLUMN_RATING)`
- `group_by!(|b: Blog| b.category_id)` → `.group_by_columns_internal(&[Blog::COLUMN_CATEGORY_ID])`
- `group_by!(|b: Blog| (b.category_id, b.author_id))` → 多列元组支持
- `sum!(|b: Blog| b.rating)` → `.sum_internal(Blog::COLUMN_RATING).await`
- `avg!(|b: Blog| b.rating)` → `.avg_internal(Blog::COLUMN_RATING).await`
- `min!(|b: Blog| b.rating)` → `.min_internal(Blog::COLUMN_RATING).await`
- `max!(|b: Blog| b.rating)` → `.max_internal(Blog::COLUMN_RATING).await`

**额外修复**:
- `min`/`max` 改为泛型返回 `EfResult<Option<V>>` where `V: FromStr + From<DbValue>`，移除残留 `<V>` 参数
- 新增 `min_internal<V>`/`max_internal<V>` 强类型版本

**验证**:
- `sqlite_crud_tests.rs` 中排序/聚合测试改用新宏
- 新增元组多列 `group_by!` 测试

### 阶段 3: 批量更新与投影 DSL

**目标**: 替换 `set_column`/`select_columns`

**改动文件**:
- [crates/macros/src/query_dsl_macros.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/query_dsl_macros.rs) — 扩展
- [crates/core/src/query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs) — `ExecuteUpdateBuilder`/`SelectQueryBuilder` 拆分

**宏清单**:
- `set!(|b: Blog| b.rating, 5)` → `.set_column_internal(Blog::COLUMN_RATING, 5)`
- `select!(|b: Blog| (b.url, b.rating))` → `.select_columns_internal(&[Blog::COLUMN_URL, Blog::COLUMN_RATING])`
- `select!(|b: Blog| b.url)` → 单列投影

**额外修复**:
- 移除 `set_property` 的 `_accessor` 死代码，或改为 `set!` 宏的底层实现
- `SelectQueryBuilder` 返回强类型元组（未来扩展，本阶段保留 `Vec<Vec<String>>`）

**验证**:
- 批量更新测试改用 `set!`
- 投影测试改用 `select!`

### 阶段 4: JOIN 与查询过滤器 DSL

**目标**: 替换 `inner_join`/`left_join`/`has_query_filter`

**改动文件**:
- [crates/macros/src/query_dsl_macros.rs](file:///E:/GitCode/RF/rust-ef/crates/macros/src/query_dsl_macros.rs)
- [crates/core/src/query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs)
- [crates/core/src/model_builder.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs)

**宏清单**:
- `inner_join!(Blog, |p: Post| p.blog_id, |b: Blog| b.blog_id)` → `.inner_join_internal(Blog::TABLE, Post::COLUMN_BLOG_ID, Blog::COLUMN_BLOG_ID)`
- `left_join!(Blog, |p: Post| p.blog_id, |b: Blog| b.blog_id)` → 同理
- `query_filter!(|b: Blog| b.deleted_at.is_null())` → 复用 `linq!` 表达式编译，生成 BoolExpr 存储而非原始 SQL

**ModelBuilder 改造**:
- `has_query_filter` 接受 `BoolExpr` 而非 `&str`
- `DbContext` 创建 DbSet 时将 `BoolExpr` 注入而非 SQL 字符串
- `QueryBuilder::apply_query_filter` 将 `BoolExpr` 合并到 `where_expr`

**注意**: `having` 暂保留原始 SQL（表达式树扩展 HAVING 支持工作量大，列入后续迭代）。

**验证**:
- JOIN 测试改用 `inner_join!`/`left_join!`
- 全局过滤器测试改用 `query_filter!`，验证 BoolExpr 注入

### 阶段 5: find_by_id 重构为终端方法

**目标**: 修复硬编码 `"id"` 列名，改为动态主键列 + 泛型主键类型 + 终端返回

**改动文件**:
- [crates/core/src/query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs)
- [crates/core/src/metadata.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/metadata.rs) — 新增 `primary_key_column() -> Option<&str>` 便捷方法

**新 API**:
```rust
impl<T: IEntityType> QueryBuilder<T> {
    /// 按主键查找，动态取主键列名，支持任意主键类型
    pub async fn find<V: Into<DbValue> + Clone>(self, key: V) -> EfResult<Option<T>> {
        let pk = T::entity_meta().primary_key_column()
            .ok_or_else(|| EfError::Metadata("No primary key".into()))?;
        self.filter_column_internal(pk, "=", key).first_or_default().await
    }

    /// 按复合主键查找
    pub async fn find_by_keys(self, keys: &[(impl AsRef<str>, impl Into<DbValue>)]) -> EfResult<Option<T>> { ... }
}
```

**废弃**:
- `find_by_id(id: i32)` 标记 `#[deprecated]`，委托 `find(id)`
- `find_by_key(HashMap<String, DbValue>)` 标记 `#[deprecated]`

**验证**:
- 自定义主键名实体（如 `blog_id` 而非 `id`）的 `find` 测试
- `i32`/`i64`/`String` 主键类型测试

### 阶段 6: LINQ 终端方法补全

**目标**: 对齐 EF Core LINQ 表达能力

**改动文件**:
- [crates/core/src/query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs)

**新增方法**:

| 方法 | 签名 | 实现要点 |
|------|------|---------|
| `last_or_default` | `async fn last_or_default(self) -> EfResult<Option<T>>` | 反向排序 + LIMIT 1；若无 order_by 则报错（语义不明） |
| `single` | `async fn single(self) -> EfResult<T>` | LIMIT 2，若 rows.len() != 1 报错 `EfError::NonUniqueResult` |
| `single_or_default` | `async fn single_or_default(self) -> EfResult<Option<T>>` | LIMIT 2，0 行 None，1 行 Some，2 行报错 |
| `to_dictionary` | `async fn to_dictionary<K, V>(self, key_sel: Fn(&T)->K, val_sel: Fn(&T)->V) -> EfResult<HashMap<K,V>>` | 先 `to_list` 再内存投影 |
| `distinct` | `fn distinct(self) -> Self` | 添加 `SELECT DISTINCT` 标志位 |
| `all` | `async fn all(self, predicate: BoolExpr) -> EfResult<bool>` | `WHERE NOT (predicate)` 后 `count == 0` |
| `contains` | `async fn contains(self, item: T) -> EfResult<bool>` where T: IGetKeyValues | 按主键 `WHERE pk = ?` 后 `any` |
| `long_count` | `async fn long_count(self) -> EfResult<i64>` | 委托 `count`（API 名称对齐） |

**EfError 扩展**:
- 新增 `EfError::NonUniqueResult { expected: usize, actual: usize }` 变体

**验证**:
- 每个新方法至少 1 个集成测试
- `single` 多行报错测试

### 阶段 7: 废弃字符串 API + 文档更新

**目标**: 完成 deprecation，文档全面更新

**改动文件**:
- [crates/core/src/query.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/query.rs) — 所有 `&str` 公开 API 加 `#[deprecated]`
- [crates/core/src/model_builder.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/model_builder.rs) — `has_query_filter` deprecate
- [crates/core/src/db_set.rs](file:///E:/GitCode/RF/rust-ef/crates/core/src/db_set.rs) — `exists_by_id` 改用 `find`
- [README.md](file:///E:/GitCode/RF/rust-ef/README.md) — 示例改用 DSL
- [docs/rust-ef/](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/) — 全部章节示例更新

**文档更新清单**:
- [docs/rust-ef/04-relationships/eager-loading.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/04-relationships/eager-loading.md) — `include!`/`then_include!`
- [docs/rust-ef/05-query-patterns/filter-sort-page.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/05-query-patterns/filter-sort-page.md) — `order_by!`
- [docs/rust-ef/05-query-patterns/linq-macro.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/05-query-patterns/linq-macro.md) — 新增 DSL 宏总览
- [docs/rust-ef/05-query-patterns/count-any.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/05-query-patterns/count-any.md) — `single`/`last_or_default`/`find`
- [docs/rust-ef/06-advanced-query/aggregation.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/06-advanced-query/aggregation.md) — `sum!`/`avg!`/`min!`/`max!`
- [docs/rust-ef/06-advanced-query/group-by-having.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/06-advanced-query/group-by-having.md) — `group_by!`
- [docs/rust-ef/06-advanced-query/join-queries.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/06-advanced-query/join-queries.md) — `inner_join!`/`left_join!`
- [docs/rust-ef/06-advanced-query/global-query-filters.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/06-advanced-query/global-query-filters.md) — `query_filter!`
- [docs/rust-ef/08-bulk-operations/execute-update.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/08-bulk-operations/execute-update.md) — `set!`
- [docs/rust-ef/11-best-practices/common-pitfalls.md](file:///E:/GitCode/RF/rust-ef/docs/rust-ef/11-best-practices/common-pitfalls.md) — 字符串 API 迁移指南
- [docs/PRODUCTION_READINESS_SPEC.md](file:///E:/GitCode/RF/rust-ef/docs/PRODUCTION_READINESS_SPEC.md) — 更新限制章节

**同步迭代计划**:
- 更新 [.trae/documents/迭代计划_v0.4_v1.0_plan.md](file:///E:/GitCode/RF/rust-ef/.trae/documents/迭代计划_v0.4_v1.0_plan.md) — 将 DSL 改造纳入 v0.4 Beta 1 核心任务

---

## 四、假设与决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| DSL 语法风格 | 闭包字段访问 | 与 `linq!` 一致，IDE 补全友好 |
| `then_include!` 类型标注 | 必须显式标注 | 链式上下文无法推断，与 `linq!` 约束一致 |
| 底层 `*_named` 方法 | 保留 `pub(crate)` + 公开版 `#[deprecated]` | 不破坏 navigation_loader 内部调用，渐进迁移 |
| `having` DSL 化 | 暂不实现 | 表达式树扩展 HAVING 工作量大，保留原始 SQL |
| `find_by_id` 重构 | 改为终端 `find` 方法 | 修复硬编码 `"id"` bug，支持任意主键类型 |
| `min`/`max` 返回类型 | 改为强类型 `Option<V>` | 修复返回 `String` 的类型安全问题 |
| `to_dictionary` 实现 | 内存投影（先 to_list） | 避免数据库端复杂聚合，简单可靠 |
| `distinct` 实现 | `SELECT DISTINCT` 标志位 | 数据库端去重，高效 |
| `all`/`contains` 实现 | 复用现有 `count`/`any` | 最小改动 |
| `#[navigation]` 属性 | 本计划不消费 | derive 宏已通过类型字符串识别导航字段，消费 `#[navigation]` 列入后续 |
| `#[foreign_key]` field 名 bug | 本计划修复 | 改为 `foreign_key_field` 真正存储字段名 |

---

## 五、风险与应对

| 风险 | 影响 | 应对 |
|------|:----:|------|
| `then_include!` 必须标注类型，体验略差 | 中 | 文档说明原因，未来可研究 `IncludeChain<T, Nav>` 类型封装 |
| `group_by!` 元组多列解析复杂 | 中 | 第一阶段只支持单列，元组支持作为增强项 |
| `query_filter!` 需扩展 ModelBuilder 存储 BoolExpr | 中 | 改动 `ModelBuilder` 数据结构，需同步 `DbContext::set::<T>()` 注入逻辑 |
| deprecation 警告淹没现有测试 | 低 | 测试同步迁移，CI 允许 deprecation 警告过渡期 |
| `find` 终端方法改变返回类型 | 中 | `find_by_id` 保留 deprecated 别名，返回 `Self` 委托 `find` |
| derive 宏生成 `FIELD_<NAME>` 可能与用户字段冲突 | 低 | 命名空间隔离（实体 impl 块内） |

---

## 六、验证步骤

### 阶段验证

每个阶段完成后执行：
1. `cargo check --workspace` 编译通过
2. `cargo test --workspace` 所有测试通过
3. `cargo clippy --workspace -- -D warnings` 零警告（deprecation 警告除外）
4. 阶段相关文档章节已更新

### 整体验收

- [ ] 所有 `&str` 公开 API 标记 `#[deprecated]`，调用时产生编译警告
- [ ] DSL 宏覆盖：`include!`/`then_include!`/`order_by!`/`order_by_desc!`/`group_by!`/`sum!`/`avg!`/`min!`/`max!`/`set!`/`select!`/`inner_join!`/`left_join!`/`query_filter!`
- [ ] LINQ 终端方法补全：`find`/`last_or_default`/`single`/`single_or_default`/`to_dictionary`/`distinct`/`all`/`contains`/`long_count`
- [ ] `find_by_id` 硬编码 `"id"` bug 修复
- [ ] `min`/`max` 返回强类型
- [ ] `#[foreign_key]` field 名 bug 修复
- [ ] 现有测试全部迁移到 DSL 宏
- [ ] 文档全部章节示例更新
- [ ] `README.md` 示例更新
- [ ] 迭代计划文档同步更新

---

## 七、与迭代计划的集成

本计划应纳入 [迭代计划_v0.4_v1.0_plan.md](file:///E:/GitCode/RF/rust-ef/.trae/documents/迭代计划_v0.4_v1.0_plan.md) 的 **v0.4 Beta 1** 阶段，作为核心任务之一：

- 阶段 1-3（导航/排序/聚合/批量更新 DSL）→ v0.4 Beta 1
- 阶段 4-5（JOIN/查询过滤器/find 重构）→ v0.4 Beta 1
- 阶段 6（LINQ 终端方法补全）→ v0.4 Beta 1
- 阶段 7（废弃字符串 API + 文档）→ v0.4 Beta 1 收尾

执行本计划后，v0.4 Beta 1 的「查询完备性」目标将真正达成，且类型安全性大幅提升。
