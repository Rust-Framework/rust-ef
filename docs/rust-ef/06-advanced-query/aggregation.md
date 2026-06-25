# 聚合函数：SUM / AVG / MIN / MAX

## 基本用法

```rust
let set = ctx.set::<OrderItem>();

let total_sales = set.query().sum("amount").await?;
let avg_price = set.query().avg("price").await?;
let min_price = set.query().min::<f64>("price").await?;
let max_price = set.query().max::<f64>("price").await?;
```

## 带过滤的聚合

```rust
let set = ctx.set::<OrderItem>();
let expr = linq!(|i: OrderItem| i.category_id == target_cat);

let filtered_sum = set.filter(expr).sum("amount").await?;
```

## 性能注意

- 聚合在**数据库端**执行，只返回一个标量值，比加载全部行再内存求和高效得多
- 对空表，SUM 返回 `0.0`，AVG 返回 `0.0`

## 设计要点

| 实践 | 说明 |
|------|------|
| 聚合前先用 `linq!` 过滤 | 减少扫描的数据量 |
| 不需要实体物化时优先用聚合 | 避免 `to_list()` 的内存和序列化开销 |

下一节：[GROUP BY 与 HAVING](group-by-having.md)
