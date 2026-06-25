# GROUP BY 与 HAVING

## GROUP BY

```rust
let report = ctx
    .set::<OrderItem>()
    .query()
    .group_by(&["category_id"])
    .to_list()
    .await?;
```

> ⚠️ 当前版本 `group_by` 返回原始行，投影到聚合列的能力需配合 `select_columns` 使用。

## 配合聚合

```rust
let set = ctx.set::<OrderItem>();

let category_totals = set
    .query()
    .group_by(&["category_id"])
    .select_columns(&["category_id", "SUM(amount) as total"])
    .to_list()
    .await?;
```

## HAVING

```rust
let result = ctx
    .set::<OrderItem>()
    .query()
    .group_by(&["category_id"])
    .having("SUM(amount) > 1000")
    .to_list()
    .await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 复杂的 GROUP BY 报表考虑存储过程 | 当 ORM 表达能力不足时，直接使用原始 SQL 更可控 |
| HAVING 用原始表达式 | 当前版本 HAVING 参数化支持有限，注意 SQL 注入风险 |

下一节：[JOIN 查询](join-queries.md)
