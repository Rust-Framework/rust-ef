# 过滤、排序与分页

所有过滤、排序、分页操作通过 `linq!` 宏表达。字符串 API（`order_by("col")` 等）已移除。

## 过滤

```rust
// 形式 A：可复用过滤闭包
let expr = linq!(|b: Blog| b.rating > 3);
let filtered = ctx.set::<Blog>().filter(expr).to_list().await?;

// 形式 B：直接查询
let filtered = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3).to_list().await?;
```

## 排序

```rust
// 形式 B 多子句：order_by 子句
let sorted = linq!(ctx.set::<Blog>(); order_by b.rating desc).to_list().await?;

// 升序（默认）
let sorted = linq!(ctx.set::<Blog>(); order_by b.rating asc).to_list().await?;

// 过滤 + 排序
let sorted = linq!(ctx.set::<Blog>(), |b: Blog| b.published;
    order_by b.created_at desc;
).to_list().await?;
```

## 分页

```rust
let page_size = 20;
let page_index = 0;

let page = linq!(ctx.set::<Blog>();
    order_by b.created_at desc;
    skip page_index * page_size;
    take page_size;
).to_list().await?;
```

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
| 形式 B source 须含 turbofish | `linq!(ctx.set::<T>(); ...)` 而非裸变量 |

下一节：[计数与存在性检查](count-any.md)
