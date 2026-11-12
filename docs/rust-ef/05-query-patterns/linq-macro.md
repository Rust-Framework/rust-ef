# `linq!` 宏：统一 DSL 入口

`linq!` 是 `rust-ef` 的**唯一查询 DSL 入口**。它在**编译期**把 Rust 闭包表达式翻译成参数化的条件树与 `QueryBuilder` 调用，覆盖过滤、排序、聚合、JOIN、分组、批量更新等全部数据库操作。

## 三种语法形式

| 形式 | 用途 | 示例 |
|------|------|------|
| **谓词** | 可复用、可组合的 WHERE 条件 → `Predicate<T>` | `linq!(\|b: Blog\| b.rating > 5)` |
| **查询** | 一次性表达完整查询 → `QueryBuilder<T>` | `linq!(ctx.set::<Blog>(); order_by b.rating desc)` |
| **模型配置** | `index` / `key` 关键字 → `&'static [&'static str]` | `linq!(index \|b: Blog\| (b.author_id, b.created_at))` |

dispatch 规则：首 token 为 `index` / `key` 关键字 → 模型配置；首 token 为 `|` → 谓词；否则按「查询」（`source[, |b: T| <cond>][; 子句]`）解析。

---

## 谓词：`linq!(|b: T| <条件>)`

```rust
// 可复用的谓词（推荐先 let 出来）
let expr = linq!(|b: Blog| b.rating > 5);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;

// 直接查询（source + 条件闭包）
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;
```

### 支持的操作符

| 操作 | 示例 | 生成的 SQL |
|------|------|-----------|
| 比较 | `b.rating > 5` | `rating > ?` |
| 等于 | `b.url == "x"` | `url = ?` |
| 不等于 | `b.active != true` | `active != ?` |
| AND | `b.rating > 5 && b.active` | `(rating > ?) AND (active = ?)` |
| OR | `b.rating > 5 \|\| b.rating < 2` | `(rating > ?) OR (rating < ?)` |
| NOT | `!(b.rating < 2)` | `NOT (rating < ?)` |
| LIKE | `b.url.contains("dot")` | `url LIKE '%dot%'` |
| StartsWith | `b.url.starts_with("https")` | `url LIKE 'https%'` |
| EndsWith | `b.url.ends_with(".com")` | `url LIKE '%.com'` |
| IN | `ids.contains(b.id)` | `id IN (?, ?, ?)` |
| LIKE（参数是变量/字面量） | `b.url.contains(keyword)` | `url LIKE ?` |
| BETWEEN | `b.rating.between(1, 5)` | `rating BETWEEN ? AND ?` |
| IS NULL | `b.content.is_null()` | `content IS NULL` |
| IS NOT NULL | `b.content.is_not_null()` | `content IS NOT NULL` |

### 组合复杂条件

```rust
let set = ctx.set::<Blog>();

let expr = linq!(|b: Blog|
    (b.rating > 5 || b.rating < 2) && b.active && b.url.contains("dotnet")
);

let blogs = set.filter(expr).to_list().await?;
```

条件需要按请求动态拼装（可选参数、OR 组、整体 AND）时，用 `Predicate<T>` 组合再 `filter` 应用，见 [过滤、排序与分页](filter-sort-page.md) 的「动态条件组合」一节。

---

## 查询：`linq!(<source>[, |b: T| <条件>]; <子句>…)`

用 `;` 分隔的子句序列表达完整查询；`source` 必须是 `QueryBuilder<T>` 或 `DbSet<T>` 表达式（含 turbofish）。

### 含过滤 + 多子句

```rust
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0.5;
    include b.posts then b.comments;
    order_by b.created_at desc;
    select (b.blog_id, b.title, b.rating);
).to_list().await?;
```

### 纯子句（无过滤）

```rust
let blogs = linq!(ctx.set::<Blog>();
    include b.posts;
    order_by b.created_at;
).to_list().await?;
```

### 聚合终端

```rust
// sum 返回 f64
let total_views: f64 = linq!(ctx.set::<Blog>(); sum b.views).await?;

// max/min 返回 Option<V>，类型由调用点推断（G1 修复后）
let top_rating: i32 = linq!(ctx.set::<Blog>(); max b.rating).await?.unwrap_or(0);
let min_views: i64 = linq!(ctx.set::<Blog>(); min b.views).await?.unwrap_or(0);

// count 返回 i64
let n: i64 = linq!(ctx.set::<Blog>(); count).await?;
```

### 全部子句一览

| 子句 | 语法 | 示例 |
|------|------|------|
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
| `take` | `take N` | `take 20` |
| `skip` | `skip N` | `skip 10` |

聚合终端（`sum`/`avg`/`min`/`max`/`count`）与 `execute_update` 是查询链终点，后续不能再链式。

### JOIN 多参数闭包

```rust
// inner join
let rows = linq!(ctx.set::<Blog>();
    inner_join |a: Blog, b: Post| a.blog_id == b.blog_id
).to_list().await?;

// left join（保留左表所有行，右表无匹配则为 NULL）
let rows = linq!(ctx.set::<Blog>();
    left_join |a: Blog, b: Post| a.blog_id == b.blog_id
).to_list().await?;
```

### 批量更新

```rust
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.rating < 0.1;
    set b.published, false;
    execute_update
).await?;
```

---

## 模型配置：`index` / `key`

用于 `ModelBuilder` / `EntityTypeBuilder` 配置，产出常量而非查询：

```rust
// 全局查询过滤器 → 产出 Predicate<Blog>（has_query_filter 也接受裸 BoolExpr）
builder.has_query_filter(linq!(|b: Blog| b.deleted_at.is_null()));

// 索引定义 → 产出 &'static [&'static str]
builder.has_index(linq!(index |b: Blog| (b.author_id, b.created_at)));

// 主键定义 → 产出 &'static [&'static str]
builder.has_key(linq!(key |b: Blog| b.blog_id));
```

`linq!(|b: T| <条件>)` 产出的谓词自带绑定值，可直接 `and` / `or` / `and_opt` 组合，再用 `QueryBuilder::filter` 应用：

```rust
let mut predicate = Predicate::<Blog>::always()
    .and(linq!(|b: Blog| b.deleted_at.is_null()))
    .and_opt(keyword.map(|kw| linq!(|b: Blog| b.url.contains(kw))));

let blogs = ctx.set::<Blog>().query().filter(predicate).to_list().await?;
```

需要裸 `BoolExpr`（例如跨 API 传递或自建条件）时用 `.into_expr()`。

---

## 推荐代码风格

建议采用「split `let` bindings」风格（非硬性约束），避免过度链式：

```rust
// 推荐：分步 let
let expr = linq!(|b: Blog| b.rating > 0.5);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;

// 推荐：复杂查询用多子句形式
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.published;
    include b.posts then b.comments;
    order_by b.created_at desc;
).to_list().await?;

// 推荐：聚合也用多子句
let total_views: f64 = linq!(ctx.set::<Blog>(); sum b.views).await?;
```

---

## 类型标注要求

当前版本 `linq!` 需要显式标注实体类型：

```rust
// ✅ 正确
let expr = linq!(|b: Blog| b.rating > 5);

// ❌ 暂不支持（类型推断规划中）
let expr = linq!(|b| b.rating > 5);
```

查询形式的 source 表达式必须含 turbofish `::<Type>`（裸变量不支持）：

```rust
// ✅ 正确：turbofish 让宏识别实体类型
linq!(ctx.set::<Blog>(); order_by b.rating desc)

// ❌ 错误：宏无法从变量推断实体类型
let set = ctx.set::<Blog>();
linq!(set; order_by b.rating desc)
```

---

## 常见错误

```rust
// ❌ 链式挤在一起难以阅读
let blogs = ctx.set::<Blog>().filter(linq!(|b: Blog| b.rating > 5 && b.active)).to_list().await?;

// ✅ 拆分为独立绑定
let expr = linq!(|b: Blog| b.rating > 5 && b.active);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;
```

```rust
// ❌ 使用已移除的字符串 API（include_named/order_by("col")/sum("col") 等均已删除）
let blogs = ctx.set::<Blog>().query().include_named("posts").to_list().await?;

// ✅ 用 linq! 多子句形式
let blogs = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;
```

---

## 设计要点

| 实践 | 说明 |
|------|------|
| 复杂条件拆分为 `let` | 提升可读性，便于调试 |
| 谓词可复用 | 同一份 `linq!(|b: T| …)` 可用于多个查询，也可 `and` / `or` 组合 |
| 注意类型标注 | 省略会导致编译错误 |
| 查询形式的 source 须含 turbofish | 裸变量不支持，宏需 `::<Type>` 识别实体 |
| 字符串 API 已全部移除 | `include_named`/`order_by("col")`/`sum("col")` 等均不存在，统一用 `linq!` |

下一节：[过滤、排序与分页](filter-sort-page.md)
