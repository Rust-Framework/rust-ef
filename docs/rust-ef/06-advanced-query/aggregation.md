# 聚合函数：SUM / AVG / MIN / MAX

聚合有两条入口：

- **`linq!` 子句**（`sum` / `avg` / `min` / `max` / `count`）——实体查询的语法糖；`sum`/`avg` 返回 `f64`。
- **运行时终端**——`ctx.set::<T>().query().sum_as::<V>(col)` / `sum_i64` / `avg_as`；列名与目标类型在运行时决定，整数聚合精确（2.7.0 新增）。

`linq!` 闭包中的 `i.amount` 在编译期解析为实体列常量（如 `OrderItem::COLUMN_AMOUNT`）；运行时 API 直接传列名常量。

## 基本用法（`linq!`，f64 契约）

```rust
// sum / avg 返回 f64（2.x 既有契约，2.7.0 保持不变）
let total_sales: f64 = linq!(ctx.set::<OrderItem>(); sum i.amount).await?;
let avg_price: f64 = linq!(ctx.set::<OrderItem>(); avg i.price).await?;

// min / max 返回 Option<V>，类型由调用点推断
let min_price: f64 = linq!(ctx.set::<OrderItem>(); min i.price).await?.unwrap_or(0.0);
let max_price: f64 = linq!(ctx.set::<OrderItem>(); max i.price).await?.unwrap_or(0.0);

// 跨类型推断：i32 列赋给 i64 变量
let max_qty: i64 = linq!(ctx.set::<OrderItem>(); max i.quantity).await?.unwrap_or(0);
```

## 类型化标量聚合（2.7.0 新增）

`sum`/`avg` 固定经过 `f64`；金额、库存、计数这类**整数**聚合在超过 2^53 后会被 `f64` 舍入。类型化聚合把 provider 返回的单元格**直接**按目标类型 `TryFrom<DbValue>` 解码，不经过 `f64`：

| 方法（`QueryBuilder<T>`） | 返回 | 空集 / 全 NULL | 用途 |
|---|---|---|---|
| `sum_as::<V>(column)` | `EFResult<V>` | 目标类型零：`V::try_from(DbValue::I64(0))` | 整数/Decimal 等精确 SUM |
| `sum_i64(column)` | `EFResult<i64>` | `0` | `sum_as::<i64>` 便捷形式 |
| `avg_as::<V>(column)` | `EFResult<V>` | 同上 | AVG 的精确解码（目标类型由调用点决定） |

```rust
use rust_ef::prelude::*;

// 整数金额：精确，2^53 以上不丢位
let total: i64 = ctx.set::<OrderItem>().query().sum_i64("amount").await?;

// 等价写法
let total: i64 = ctx
    .set::<OrderItem>()
    .query()
    .sum_as::<i64>("amount")
    .await?;

// 明确需要浮点结果时
let total: f64 = ctx
    .set::<OrderItem>()
    .query()
    .sum_as::<f64>("amount")
    .await?;

// AVG：目标类型决定解码
let avg: f64 = ctx
    .set::<OrderItem>()
    .query()
    .avg_as::<f64>("price")
    .await?;
```

精度示例（整数列，两行分别为 `2^53 + 1` 与 `2`）：

```rust
let exact: i64 = ctx.set::<Counter>().query().sum_as::<i64>("amount").await?;
assert_eq!(exact, 9_007_199_254_740_995); // 2^53 + 3，精确

// 同一结果的 f64 路径会发生舍入：整数聚合不要用 f64
let lossy: f64 = ctx.set::<Counter>().query().sum_as::<f64>("amount").await?;
assert_ne!(lossy as i64, exact);
```

### 目标类型怎么选

| 列语义 | 目标类型 | 说明 |
|---|---|---|
| 整数金额 / 库存 / 计数 | `i64`（`sum_i64`）或其它可 `TryFrom<DbValue>` 的整数 | 不经过 `f64`，无 2^53 精度损失 |
| 需要小数的精确金额 | `rust_decimal::Decimal`（`decimal` feature） | 是否可解码取决于 provider 返回的单元格：Decimal/字符串/整数单元格可解析，REAL 单元格（如 SQLite 的 `AVG`）不可 |
| 统计 / 比率 / 兼容既有契约 | `f64` | 与 `linq!` 的 `sum`/`avg` 一致 |

> `avg_as` 的精确程度由**目标类型 + provider 返回的单元格**共同决定：SQLite 的 `AVG` 始终返回 REAL，所以 `avg_as::<i64>` 会得到 typed `EFError::TypeConversion`；PostgreSQL/MySQL 的 `SUM`/`AVG` 返回 numeric/DECIMAL——2.7 起整值 NUMERIC 可解码为整数目标（非整值报 `TypeConversion`，不静默截断），任意值可解码为 `Decimal` 或 `f64`。

## 空集语义（EF-17，2.7.0 不变）

SQL 对空输入集的聚合仍会返回一行，单元格为 `NULL`。`rust-ef` 把该 `NULL` 归一为**中性值**：

| 聚合 | 空集 / 全 NULL 列 | 说明 |
|---|---|---|
| `sum`（`linq!`） | `0.0` | 与 SQL「空集 SUM = 空和」语义一致 |
| `avg`（`linq!`） | `0.0` | SQL 中空集 AVG 是 `NULL`；`rust-ef` 文档化归一为中性值 `0.0` |
| `sum_as` / `sum_i64` / `avg_as` | 目标类型的零（由 `DbValue::I64(0)` 解码） | 例如 `i64` → `0`、`f64` → `0.0` |
| `min` / `max` | `Ok(None)` | 签名是 `Option<V>`，SQL NULL 转为 `None` |
| `count` | `0` | 数行数，不经过 NULL 单元格归一 |

```rust
// 表为空：不报错，返回 0.0
let total: f64 = linq!(ctx.set::<OrderItem>(); sum i.amount).await?;
assert_eq!(total, 0.0);

// 类型化聚合同样是目标类型零
let total: i64 = ctx.set::<OrderItem>().query().sum_i64("amount").await?;
assert_eq!(total, 0);

// 过滤后为空同理
let filtered: f64 = linq!(ctx.set::<OrderItem>().query(), |i: OrderItem| i.price > 1000.0;
    avg i.price
).await?;
assert_eq!(filtered, 0.0);
```

> 归一发生在**结果层**（单元格转换），SQL 不做 `COALESCE` 改写，因此三种方言（SQLite / PostgreSQL / MySQL）行为一致，非空结果逐值不变。
> 分组的「组内 NULL」是另一套语义：投影里的 `SUM`/`AVG` 不做隐式 COALESCE，需要显式中性值时用 `SelectItem::sum(col).coalesce(0)`（见 [类型化投影](typed-projections.md)）。

## 带过滤的聚合

```rust
// linq! 多子句形式：过滤 + 聚合终端
let filtered_sum: f64 = linq!(ctx.set::<OrderItem>(), |i: OrderItem| i.category_id == target_cat;
    sum i.amount
).await?;

// 运行时 API：Predicate 复用同一套动态条件
let mut predicate = Predicate::<OrderItem>::always();
predicate = predicate.and_opt(min_price.map(|p| linq!(|i: OrderItem| i.price >= p)));
let total: i64 = ctx
    .set::<OrderItem>()
    .query()
    .filter(predicate)
    .sum_i64("amount")
    .await?;
```

## 性能注意

- 聚合在**数据库端**执行，只返回一个标量值，比加载全部行再内存求和高效得多
- `min` / `max` 通过 `TryFrom<DbValue>` 在调用点保留原类型，无需手动 `parse`
- 整数聚合优先 `sum_i64`/`sum_as::<i64>`：既精确，又避免一次 `f64` 转换

## 设计要点

| 实践 | 说明 |
|------|------|
| 聚合前先过滤 | 减少扫描的数据量 |
| 不需要实体物化时优先用聚合 | 避免 `to_list()` 的内存和序列化开销 |
| 金额/库存等整数聚合用 `sum_as::<i64>`/`sum_i64` | `f64` 在 2^53 后丢失整数精度 |
| `min`/`max` 类型由调用点推断 | `let v: i64 = linq!(...; max b.col).await?.unwrap_or(0);` |
| `avg_as` 目标类型要匹配 provider 单元格 | SQLite `AVG` 返回 REAL，用 `f64`；Decimal 需 provider 返回可解析单元格 |

下一节：[GROUP BY 与 HAVING](group-by-having.md)
