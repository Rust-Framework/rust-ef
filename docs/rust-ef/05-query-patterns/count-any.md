# 计数与存在性检查

## count

```rust
// 链式调用
let total = ctx.set::<Blog>().query().count().await?;

// linq! 多子句形式
let n: i64 = linq!(ctx.set::<Blog>(); count).await?;
```

生成 `SELECT COUNT(*)`，比加载全部实体再 `len()` 高效得多。

## long_count

```rust
// long_count 是 count 的 i64 别名（Rust 中两者均返回 i64）
let n: i64 = ctx.set::<Blog>().query().long_count().await?;
```

## any

```rust
let exists = linq!(ctx.set::<Blog>(), |b: Blog| b.url.contains("dotnet"))
    .any()
    .await?;

if exists {
    println!("Found matching blogs");
}
```

生成 `SELECT 1 ... LIMIT 1`，是最轻量的存在性检查方式。

## first / first_or_default

```rust
// find 返回 Option<T>，找不到时为 None（使用实体 PK 元数据）
let blog = ctx.set::<Blog>().query().find(1).await?;

// first 找不到时返回 EfError::NotFound
let blog = ctx.set::<Blog>().query().first().await?;

// first_or_default 找不到时返回 None
let maybe = ctx.set::<Blog>().query().first_or_default().await?;
```

## last / last_or_default

```rust
// 无显式排序时，按主键倒序取最后一条
let last = ctx.set::<Blog>().query().last().await?;
let maybe_last = ctx.set::<Blog>().query().last_or_default().await?;

// 有显式排序时，反转方向后取首行
let last = linq!(ctx.set::<Blog>(); order_by b.rating asc)
    .last()
    .await?;
```

## single / single_or_default

```rust
// single：有且仅有一条，否则报错
let only = ctx.set::<Blog>().query().single().await?;

// single_or_default：0 或 1 条，否则报错
let maybe_only = ctx.set::<Blog>().query().single_or_default().await?;
```

## contains

```rust
// 检查是否存在指定主键的实体
let has_blog = ctx.set::<Blog>().query().contains(1).await?;
```

## all

```rust
// 是否全部满足谓词（谓词在 Rust 侧应用）
let all_published = ctx.set::<Blog>().query().all(|b| b.published).await?;
```

## 复合主键查找

```rust
use rust_ef::provider::DbValue;

let line = ctx
    .set::<OrderLine>()
    .query()
    .find_by_key(&[
        (OrderLine::COLUMN_ORDER_ID, DbValue::I32(1)),
        (OrderLine::COLUMN_LINE_NO, DbValue::I32(2)),
    ])
    .await?;
```

## exists_by_id / exists_by_key

`exists_by_id` 检查指定主键的行是否存在，生成 `SELECT 1 ... LIMIT 1`，比 `find(id).is_some()` 更轻量（不物化整行）。

```rust
// 单主键：读取实体 PK 元数据，无需硬编码列名
let exists = ctx.set::<Blog>().query().exists_by_id(1).await?;

// 复合主键
let exists = ctx
    .set::<OrderLine>()
    .query()
    .exists_by_key(&[
        (OrderLine::COLUMN_ORDER_ID, DbValue::I32(1)),
        (OrderLine::COLUMN_LINE_NO, DbValue::I32(2)),
    ])
    .await?;
```

`exists_by_id` 从 `T::entity_meta()` 解析主键列名，与 `find(id)` 一致；`exists_by_key` 接受 `&[(&str, DbValue)]`，与 `find_by_key` 对应。两者均委托 `any()`，仅检查行存在性而不加载实体。

## 设计要点

| 实践 | 说明 |
|------|------|
| 用 `any()` 替代 `count() > 0` | `any()` 在找到第一条后就停止，更轻量 |
| 用 `first_or_default()` 处理可能不存在的记录 | 避免 try-catch 模式 |
| `last()` 无显式排序时按 PK 倒序 | 确定性语义，与原设计一致 |
| `single()` 用 `take(2)` 校验唯一性 | 不需要 `QueryBuilder: Clone` |

下一章：[高级查询](../06-advanced-query/INDEX.md)
