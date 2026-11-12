# 过滤、排序与分页

所有过滤、排序、分页操作通过 `linq!` 宏表达。字符串 API（`order_by("col")` 等）已移除。

## 过滤

```rust
// 形式 A：可复用过滤闭包
let expr = linq!(|b: Blog| b.rating > 3);
let filtered = ctx.set::<Blog>().filter(expr).to_list().await?;

// 形式 B：直接查询
let filtered = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3).to_list().await?;
```

## 动态条件组合：`Predicate`（PredicateBuilder 风格）

列表、搜索、导出这类接口的条件多半来自可选参数。逐条 `if let` 追加虽然可行，但一旦出现「A 或 B」「整体 AND 一个 OR 组」就开始难写。`Predicate<T>` 把 `linq!(filter ...)` 产出的表达式树变成可自由组合的值（对应 EF Core 的 `PredicateBuilder` / `LinqKit`）：

```rust
use rust_ef::prelude::*;                                  // Predicate 已在 prelude 中

let mut predicate = Predicate::<Product>::always();       // AND 链起点，本身不产生 SQL

if let Some(keyword) = keyword {
    // Name 或 Description 命中关键字：OR 组作为一个整体与外部 AND
    predicate = predicate.and(
        linq!(filter |p: Product| p.name.contains(keyword))
            .or(linq!(filter |p: Product| p.description.contains(keyword))),
    );
}
if let Some(min_price) = min_price {
    predicate = predicate.and(linq!(filter |p: Product| p.price >= min_price));
}

let query = ctx.set::<Product>().query().where_(predicate);
let total = query.clone().count().await?;                 // 与列表共用同一份条件
let rows = query
    .order_by_desc_column(column!(Product::created_at))
    .take(20)
    .to_list()
    .await?;
```

| 写法 | 语义 |
|------|------|
| `Predicate::<T>::always()` | `AND` 链起点：`always().and(p) == p`，不会生成 `1=1` |
| `Predicate::<T>::never()` | `OR` 链起点：`never().or(p) == p`，用于「若干可选 OR 条件」 |
| `.and(x)` / `.or(x)` / `.not()` | 任意嵌套，`OR` 组自带括号；`x` 可以是 `BoolExpr` 或 `Predicate<T>` |
| `QueryBuilder::where_(p)` | 以 `AND` 追加到已有条件（含全局过滤器）；值按表达式树顺序绑定 |
| `p.into_expr()` / `expr.into()` | 与 `has_query_filter` 等接收 `BoolExpr` 的 API 互转 |

规则与陷阱：

- **租户/软删除等安全条件仍由服务端注入**，且放在最前面；`predicate` 只承载请求里声明过的白名单字段。
- **`count` 与列表必须共用同一个 predicate**：`QueryBuilder` 可 `clone()`，先 clone 出 `count`，再对原查询排序/分页。
- **`and` / `or` 参数直接写 `linq!(filter ...)` 即可**：`BoolExpr → Predicate<T>` 有 `From` 实现，不必手工包装。
- **`contains` 的两种含义**：`ids.contains(b.id)` 是 `IN`（参数是**列**），`b.url.contains(keyword)` 是 `LIKE`（参数是**值**）。
- **`where_` 与 `filter` 可混用**：两者都是追加 `AND`，`filter` 接收 `linq!(|b: T| ...)` 闭包，`where_` 接收组合好的树。

## 排序

```rust
// 形式 B 多子句：order_by 子句
let sorted = linq!(ctx.set::<Blog>(); order_by b.rating desc).to_list().await?;

// 升序（默认）
let sorted = linq!(ctx.set::<Blog>(); order_by b.rating asc).to_list().await?;

// 过滤 + 排序
let sorted = linq!(ctx.set::<Blog>(), |b: Blog| b.published;
    order_by b.created_at desc;
).to_list().await?;
```

## 分页

```rust
let page_size = 20;
let page_index = 0;

let page = linq!(ctx.set::<Blog>();
    order_by b.created_at desc;
    skip page_index * page_size;
    take page_size;
).to_list().await?;
```

## 组合示例

```rust
let posts = linq!(ctx.set::<Post>(), |p: Post| p.blog_id == target_blog_id;
    order_by p.post_id desc;
    skip 0;
    take 10;
).to_list().await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 先过滤再分页 | 减少排序和分页的数据量 |
| `skip` + `take` 一起用 | 数据库支持 OFFSET + LIMIT，比内存分页高效 |
| 形式 B source 须含 turbofish | `linq!(ctx.set::<T>(); ...)` 而非裸变量 |

下一节：[计数与存在性检查](count-any.md)
