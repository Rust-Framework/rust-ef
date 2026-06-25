# linq! 宏：推荐写法与可读性

`linq!` 是 `rust-ef` 的核心特性，它在**编译期**将 Rust 闭包表达式树翻译为参数化的 `QueryBuilder` 链式调用。

## 基本语法

```rust
// 可复用的表达式（推荐）
let expr = linq!(|b: Blog| b.rating > 5);
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;
```

## 支持的操作符

| 操作 | 示例 | 生成的 SQL |
|------|------|-----------|
| 比较 | `b.rating > 5` | `rating > ?` |
| 等于 | `b.url == "x"` | `url = ?` |
| 不等于 | `b.active != true` | `active != ?` |
| AND | `b.rating > 5 && b.active` | `(rating > ?) AND (active = ?)` |
| OR | `b.rating > 5 \|\| b.rating < 2` | `(rating > ?) OR (rating < ?)` |
| NOT | `!(b.rating < 2)` | `NOT (rating < ?)` |
| LIKE | `b.url.contains("dot")` | `url LIKE '%dot%'` |
| StartsWith | `b.url.starts_with("https")` | `url LIKE 'https%'` |
| EndsWith | `b.url.ends_with(".com")` | `url LIKE '%.com'` |
| IN | `ids.contains(b.id)` | `id IN (?, ?, ?)` |
| BETWEEN | `b.rating.between(1, 5)` | `rating BETWEEN ? AND ?` |
| IS NULL | `b.content.is_null()` | `content IS NULL` |
| IS NOT NULL | `b.content.is_not_null()` | `content IS NOT NULL` |

## 排序语法

```rust
// 升序
let expr = linq!(|b: Blog| b.active => b.url);

// 降序（前面加负号）
let expr = linq!(|b: Blog| b.active => -b.rating);
```

## 组合复杂条件

```rust
let set = ctx.set::<Blog>();

let expr = linq!(|b: Blog|
    (b.rating > 5 || b.rating < 2) && b.active && b.url.contains("dotnet")
);

let blogs = set.filter(expr).to_list().await?;
```

## 类型标注要求

当前版本 `linq!` 需要显式标注实体类型：

```rust
// ✅ 正确
let expr = linq!(|b: Blog| b.rating > 5);

// ❌ 暂不支持（类型推断规划中）
let expr = linq!(|b| b.rating > 5);
```

## 常见错误

```rust
// ❌ 链式挤在一起难以阅读
let blogs = ctx.set::<Blog>().filter(linq!(|b: Blog| b.rating > 5 && b.active)).to_list().await?;

// ✅ 拆分为独立绑定
let set = ctx.set::<Blog>();
let expr = linq!(|b: Blog| b.rating > 5 && b.active);
let blogs = set.filter(expr).to_list().await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 复杂条件拆分为 `let` | 提升可读性，便于调试 |
| 表达式可复用 | 同一份 `linq!` 可用于多个查询 |
| 注意类型标注 | 省略会导致编译错误 |

下一节：[过滤、排序与分页](filter-sort-page.md)
