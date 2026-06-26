# 聚合函数：SUM / AVG / MIN / MAX

聚合通过 `linq!` 宏的 `sum` / `avg` / `min` / `max` 子句表达。列字段在闭包中以 `i.amount` 形式访问（编译期解析为 `OrderItem::COLUMN_AMOUNT` 常量）。

## 基本用法

```rust
// sum / avg 返回 f64
let total_sales: f64 = linq!(ctx.set::<OrderItem>(); sum i.amount).await?;
let avg_price: f64 = linq!(ctx.set::<OrderItem>(); avg i.price).await?;

// min / max 返回 Option<V>，类型由调用点推断（G1 修复后保留原类型）
let min_price: f64 = linq!(ctx.set::<OrderItem>(); min i.price).await?.unwrap_or(0.0);
let max_price: f64 = linq!(ctx.set::<OrderItem>(); max i.price).await?.unwrap_or(0.0);

// 跨类型推断：i32 列赋给 i64 变量
let max_qty: i64 = linq!(ctx.set::<OrderItem>(); max i.quantity).await?.unwrap_or(0);
```

## 带过滤的聚合

```rust
// linq! 多子句形式：过滤 + 聚合终端
let filtered_sum: f64 = linq!(ctx.set::<OrderItem>(), |i: OrderItem| i.category_id == target_cat;
    sum i.amount
).await?;
```

## 空集语义

- `sum` / `avg` 对空表返回 `0.0`
- `min` / `max` 对空表返回 `Ok(None)`（SQL NULL 经 `convert_aggregate_cell` 转换为 `None`）

## 性能注意

- 聚合在**数据库端**执行，只返回一个标量值，比加载全部行再内存求和高效得多
- `min` / `max` 通过 `TryFrom<DbValue>` 在调用点保留原类型，无需手动 `parse`

## 设计要点

| 实践 | 说明 |
|------|------|
| 聚合前先用 `linq!` 过滤 | 减少扫描的数据量 |
| 不需要实体物化时优先用聚合 | 避免 `to_list()` 的内存和序列化开销 |
| `min`/`max` 类型由调用点推断 | 用 `let v: i64 = linq!(...; max b.col).await?.unwrap_or(0);` 显式标注 |

下一节：[GROUP BY 与 HAVING](group-by-having.md)
