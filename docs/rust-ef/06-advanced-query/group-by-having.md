# GROUP BY 与 HAVING

`group_by` 与 `having` 通过 `linq!` 宏的子句表达。字符串 API（`group_by(&[...])` / `having("...")`）已移除。

## GROUP BY

```rust
// 按单字段分组
let report = linq!(ctx.set::<OrderItem>(); group_by i.category_id)
    .to_list()
    .await?;

// 按多字段分组（元组）
let report = linq!(ctx.set::<OrderItem>(); group_by (i.category_id, i.warehouse_id))
    .to_list()
    .await?;
```

## 配合 select 投影

```rust
// group_by + select 组合
let rows: Vec<Vec<String>> = linq!(ctx.set::<OrderItem>();
    group_by i.category_id;
    select (i.category_id, i.amount);
).to_list().await?;
```

> 当前版本 `select` 返回 `Vec<Vec<String>>`（原始行数据）。强类型元组投影（返回 `(i32, f64)` 等）规划在后续版本。

## HAVING

```rust
// having 聚合表达式：agg(col) op value
let result = linq!(ctx.set::<OrderItem>();
    group_by i.category_id;
    having count(i.order_item_id) > 5;
).to_list().await?;

// 支持 sum / avg / min / max / count 五种聚合函数与简单比较运算符
let big_categories = linq!(ctx.set::<OrderItem>();
    group_by i.category_id;
    having sum(i.amount) > 1000;
).to_list().await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 复杂的 GROUP BY 报表考虑存储过程 | 当 ORM 表达能力不足时，直接使用原始 SQL 更可控 |
| `having` 首版仅支持 `agg(col) op value` | 嵌套表达式后续扩展 |
| `group_by` 接受单字段或元组 | `group_by i.cat` 或 `group_by (i.cat, i.author)` |

下一节：[JOIN 查询](join-queries.md)
