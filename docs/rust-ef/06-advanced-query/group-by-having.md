# GROUP BY 与 HAVING

分组有两种写法：

1. **运行时报表 API（2.7.0 起，推荐）**：`group_by(&[&str])` + `select_items(&[SelectItem])` + `to_list_rows::<R>()`，得到「每组一行」的类型化结果，不需要把报表伪装成实体。
2. **`linq!` 子句（可选语法糖）**：`group_by ...; having ...` 终端返回实体行（`to_list`）；`select` 里带聚合项时返回类型化投影（`to_list_typed_*` / `to_list_rows`）。

## 运行时报表 API

```rust
use rust_ef::query::SelectItem;

let rows: Vec<(String, i64)> = ctx
    .set::<OrderItem>()
    .query()
    .group_by(&["category"])
    .select_items(&[
        SelectItem::column("category"),
        SelectItem::sum("amount"),
    ])
    .order_by_desc("sum_amount") // 可按投影别名排序
    .to_list_rows::<(String, i64)>()
    .await?;
```

生成的 SQL：

```sql
SELECT category AS category, SUM(amount) AS sum_amount
FROM order_items
GROUP BY category
ORDER BY sum_amount DESC
```

要点：

- 组键列要**自己投影**（`SelectItem::column`）；投影顺序就是结果行布局，`to_list_rows` 按位置映射（元组或 `#[derive(FromRow)]`）。
- 非聚合列必须出现在 GROUP BY —— 这是 SQL 校验，框架不做二次语义检查。
- `group_bys` 非空 + 聚合项 → 每组一行；`group_bys` 非空 + 只有列项 → 组键行（等价 DISTINCT 组键）；`group_bys` 为空 + 聚合项 → 整集合一行。
- `group_by` 替换整个 GROUP BY 列表（不是追加）；`take`/`skip`/`distinct`/`order_by`/`order_by_desc` 与 `select_items` 的先后顺序任意。
- 默认聚合别名：`sum_<列名>` / `avg_<列名>` / `min_<列名>` / `max_<列名>` / `count` / `count_<列名>`；别名可被 `ORDER BY` 引用。完整规则见 [类型化投影](typed-projections.md)。

## HAVING

运行时 API 用 `HavingExpr`（与 `linq!` 的 having 子句同一套 AST）：

```rust
use rust_ef::provider::DbValue;
use rust_ef::query::{AggKind, CompareOp, HavingExpr, SelectItem};

let rows: Vec<(String, i64)> = ctx
    .set::<OrderItem>()
    .query()
    .group_by(&["category"])
    .select_items(&[
        SelectItem::column("category"),
        SelectItem::sum("amount"),
    ])
    .having(HavingExpr::Compare {
        agg: AggKind::Sum,
        col: "amount".to_string(),
        op: CompareOp::Gt,
        value: DbValue::I64(1000),
    })
    .to_list_rows::<(String, i64)>()
    .await?;
```

`HavingExpr` 的节点：

| 节点 | SQL |
|---|---|
| `Compare { agg, col, op, value }` | `AGG(col) op ?` |
| `CompareAgg { left_agg, left_col, op, right_agg, right_col }` | `AGG(c1) op AGG(c2)`（无绑定参数） |
| `And` / `Or` / `Not` | 布尔组合，可嵌套 |

多次 `having(...)` 之间自动用 `AND` 连接。绑定参数在生成 SQL 时按 **WHERE → HAVING** 的文本顺序统一收集，所以即使先 `having` 后 `filter` 也不会参数错位（2.7.0 修复）。

### linq! 可选写法

> ⚠️ 带 `group_by`/`having` 的查询**不能**再用实体的 `to_list()`/`count()`：分组结果每组
> 一行、不是实体形状（`SELECT * ... GROUP BY` 在 PG/MySQL 报错、SQLite 返回组内任意行）。
> 实体终端会返回 `EFError::Configuration`；请用聚合投影 + `to_list_typed_*`/`to_list_rows`。

```rust
// 分组聚合投影：select 里写聚合项，终端返回类型化投影
let rows: Vec<(String, i64, i64)> = linq!(ctx.set::<Blog>();
    group_by b.category;
    select (b.category, sum(b.views), count())
).to_list_typed_3::<String, i64, i64>().await?;
```

> `linq!` 的 `select` 已支持 `sum/avg/min/max(...)`、`count()`、`count(col)` 聚合项
> （纯列 `select` 的 SQL 与 2.6 逐字符一致）。`linq!` 的 `order_by` 只能引用列或组键，
> **不能按聚合别名排序**；需要「按 SUM 降序取 Top N」时用运行时
> `select_items(...).order_by_desc("sum_amount")`（默认别名规则见
> [类型化投影](typed-projections.md)）。

## 组内聚合的 NULL 语义

投影聚合**不做隐式 COALESCE**：

- `SUM`/`AVG` 在「全 NULL 组」、`MIN`/`MAX` 在空组都是 SQL NULL；
- 类型化解码遇到 NULL 会返回带列序号的 `EFError::TypeConversion`；
- 需要中性值时显式 `SelectItem::sum("amount").coalesce(0)`，生成 `COALESCE(SUM(amount), ?)`。

```rust
// 全 NULL 组 → 0，而不是 TypeConversion
select_items(&[
    SelectItem::column("category"),
    SelectItem::sum("amount").coalesce(0i64),
])
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 报表「每组一行」用运行时 API | `group_by` + `select_items` + `to_list_rows`，类型由终端决定 |
| 复杂报表不要强塞进实体 | 投影行用 `#[derive(FromRow)]` 结构体或元组，见 [类型化投影](typed-projections.md) |
| 非聚合列必须进 GROUP BY | SQL 校验；投影顺序即结果列顺序 |
| 组内 NULL 显式 COALESCE | 框架不隐式改写聚合结果 |
| `select` 与 `group_by`/`having`/`order_by` 顺序任意 | 链式方法在 `QueryBuilder` 与 `SelectQueryBuilder` 上都可用 |

下一节：[类型化投影与报表行](typed-projections.md)
