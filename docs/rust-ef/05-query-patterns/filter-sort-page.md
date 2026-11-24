# 过滤、排序与分页

所有过滤、排序、分页操作通过 `linq!` 宏表达；不要使用字符串列名 API（`order_by("col")` 等）。
排序列优先用实体 derive 生成的类型化句柄 `Entity::field`（拼错/改名编译期失败）；需要列名字符串
的命名空间 API（`set_column`/`select_item` 等）仍用 `column!`。

## 过滤

```rust
// 可复用的谓词
let expr = linq!(|b: Blog| b.rating > 3);
let filtered = ctx.set::<Blog>().filter(expr).to_list().await?;

// 直接查询（source + 条件）
let filtered = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3).to_list().await?;
```

## 动态条件组合：`Predicate`（PredicateBuilder 风格）

列表、搜索、导出这类接口的条件多半来自可选参数。逐条 `if let` 追加虽然可行，但一旦出现「A 或 B」「整体 AND 一个 OR 组」就开始难写。`Predicate<T>` 是 `linq!(|b: T| …)` 的返回值：一棵可自由组合的条件树（对应 EF Core 的 `PredicateBuilder` / `LinqKit`）：

```rust
use rust_ef::prelude::*;                                  // Predicate 已在 prelude 中

// 必需条件
let mut predicate = Predicate::<Product>::always()        // AND 链起点，本身不产生 SQL
    .and(linq!(|p: Product| !p.is_deleted));

// 可选条件：None 直接跳过，不产生 SQL，也不需要 if let
predicate = predicate.and_opt(keyword.map(|keyword| {
    // Name 或 Description 命中关键字：OR 组作为一个整体与外部 AND
    linq!(|p: Product| p.name.contains(keyword))
        .or(linq!(|p: Product| p.description.contains(keyword)))
}));
predicate = predicate.and_opt(
    min_price.map(|min_price| linq!(|p: Product| p.price >= min_price)),
);

let query = ctx.set::<Product>().query().filter(predicate);
let total = query.clone().count().await?;                 // 与列表共用同一份条件
let rows = query
    .order_by_desc(column!(Product::created_at))
    .take(20)
    .to_list()
    .await?;
```

| 写法 | 语义 |
|------|------|
| `Predicate::<T>::always()` | `AND` 链起点：`always().and(p) == p`，不会生成 `1=1` |
| `Predicate::<T>::never()` | `OR` 链起点：`never().or(p) == p`，用于「若干可选 OR 条件」 |
| `.and(x)` / `.or(x)` / `.not()` | 任意嵌套，`OR` 组自带括号；`x` 可以是 `BoolExpr` 或 `Predicate<T>` |
| `.and_opt(o)` / `.or_opt(o)` | `o: Option<_>`；`None` 不产生 SQL，可选条件不必写 `if let` |
| `QueryBuilder::filter(p)` | 以 `AND` 追加到已有条件（含全局过滤器）；值按表达式树顺序绑定 |
| `p.into_expr()` / `BoolExpr ⇄ Predicate<T>` | 与 `has_query_filter` 等接收表达式的 API 互转 |

规则与陷阱：

- **租户/软删除等安全条件仍由服务端注入**，且放在最前面；`predicate` 只承载请求里声明过的白名单字段。
- **`count` 与列表必须共用同一个 predicate**：`QueryBuilder` 可 `clone()`，先 clone 出 `count`，再对原查询排序/分页。
- **`and` / `or` 参数直接写 `linq!(...)` 即可**：它本身已经是 `Predicate<T>`；`BoolExpr` 也能通过 `From` 混入，不必手工包装。
- **可选条件用 `and_opt` / `or_opt`**：`predicate.and_opt(if cond { Some(p) } else { None })`，`None` 不产生 SQL，无需 `if let` 重新赋值。
- **`contains` 的两种含义**：`ids.contains(b.id)` 是 `IN`（参数是**列**），`b.url.contains(keyword)` 是 `LIKE`（参数是**值**）。
- **`filter` 只有一个入口**：接收 `Predicate<T>`（`linq!(|b: T| ...)` 直出）或裸 `BoolExpr`，都是追加 `AND`。

## 排序

```rust
// 纯子句查询：order_by 子句
let sorted = linq!(ctx.set::<Blog>(); order_by b.rating desc).to_list().await?;

// 升序（默认）
let sorted = linq!(ctx.set::<Blog>(); order_by b.rating asc).to_list().await?;

// 过滤 + 排序
let sorted = linq!(ctx.set::<Blog>(), |b: Blog| b.published;
    order_by b.created_at desc;
).to_list().await?;

// 类型化列句柄（2.9.1+）：拼错/字段改名在编译期失败
let sorted = ctx
    .set::<Blog>()
    .query()
    .order_by_field(Blog::created_at)
    .then_by_field_desc(Blog::id) // 末位唯一列，翻页稳定
    .to_list()
    .await?;
```

## 分页

手动分页用 `skip` + `take`：

```rust
let page_size = 20;
let page_index = 0;

let page = linq!(ctx.set::<Blog>();
    order_by b.created_at desc;
    skip page_index * page_size;
    take page_size;
).to_list().await?;
```

### 列表端点：`to_page` 直接返回 `PagedResult`

列表端点同时需要「总数 + 当前页」。`QueryBuilder::to_page(&PagedRequest)` 一次调用完成两件事并直接返回 `PagedResult<T>` 信封（`PagedRequest` / `PagedResult` 是 `rust-core` 的契约类型，rust-webx 3.0 直接重导出，Handler 可原样返回）：

```rust
use rust_core::PagedResult;   // 实体本身不需要 Serialize，只有映射后的 DTO 页需要

let page = ctx
    .set::<Order>()
    .query()
    .filter(linq!(|o: Order| o.tenant_id == tenant_id))   // count 与列表共用同一条过滤链
    .order_by_field_desc(Order::created_at)
    .then_by_field_desc(Order::id)                        // 末位唯一列，翻页稳定
    .to_page(&req.paging)                                 // PagedRequest { page, page_size }
    .await?;

// 返回的是实体页：端点用 map_items 映射 DTO 后再返回
let page: PagedResult<OrderModel> = page.map_items(OrderModel::from);
Ok(page)
```

- **两条 SQL，同一过滤**：先 `COUNT(*)`（克隆后去掉 ORDER BY/skip/take，保留 FROM/JOIN/WHERE），再按 `LIMIT/OFFSET` 取本页；`total` 与列表天然同源。两条语句不是快照——并发写入时 `total` 与 `items` 可能来自不同瞬间，需要一致响应时把调用包进显式事务。
- **直接返回信封**：`PagedResult<T>` 是扁平业务信封 `{success,status,message,items,total}`（组合 `OperationResult<()>`，Rust 没有继承）；默认 `success=true/status=0/message="ok"`，端点可用 `.with_status(..)` / `.with_message(..)` 覆盖。没有 page/page_size/total_pages 回声；`total_pages` 由客户端 `ceil(total / page_size)` 计算。
- **越界页**：`items` 为空、`total` 照实，不报错；`page`/`page_size` 的合法范围由**绑定层**校验（越界请求在绑定层 400），`to_page` 不做夹紧。
- **只有一个入口**：没有 `(Vec<T>, total)` tuple 版本，也没有 `count: bool` 开关；不需要计数时直接用 `to_list()`。
- **排序仍由调用方给**：`to_page` 只负责分页，ORDER BY 要链在调用之前；末位列用唯一列（如主键），否则并发写入时相邻页可能重复或漏行。
- **不支持的形状**：GROUP BY/HAVING、`distinct()`、聚合/窗口投影、集合运算无法定义 total，返回类型化 `EFError::Configuration`。

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
| 列表端点用 `to_page` | 一次返回 `PagedResult`，count 与列表共用过滤链；不要手写 clone + count + skip/take |
| 查询形式的 source 须含 turbofish | `linq!(ctx.set::<T>(); ...)` 而非裸变量 |

下一节：[计数与存在性检查](count-any.md)
