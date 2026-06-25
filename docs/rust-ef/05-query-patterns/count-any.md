# 计数与存在性检查

## count

```rust
let total = ctx.set::<Blog>().query().count().await?;
```

生成 `SELECT COUNT(*)`，比加载全部实体再 `len()` 高效得多。

## any

```rust
let set = ctx.set::<Blog>();
let expr = linq!(|b: Blog| b.url.contains("dotnet"));
let exists = set.filter(expr).any().await?;

if exists {
    println!("Found matching blogs");
}
```

生成 `SELECT 1 ... LIMIT 1`，是最轻量的存在性检查方式。

## first / first_or_default

```rust
// 找不到时返回 EfError::NotFound
let blog = ctx.set::<Blog>().query().find_by_id(1).first().await?;

// 找不到时返回 None
let maybe = ctx.set::<Blog>().query().find_by_id(999).first_or_default().await?;
```

## 复合主键查找

```rust
use std::collections::HashMap;
use rust_ef::provider::DbValue;

let mut keys = HashMap::new();
keys.insert("order_id".into(), DbValue::I32(1));
keys.insert("line_no".into(), DbValue::I32(2));

let line = ctx
    .set::<OrderLine>()
    .query()
    .find_by_key(&keys)
    .first()
    .await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 用 `any()` 替代 `count() > 0` | `any()` 在找到第一条后就停止，更轻量 |
| 用 `first_or_default()` 处理可能不存在的记录 | 避免 try-catch 模式 |

下一章：[高级查询](../06-advanced-query/INDEX.md)
