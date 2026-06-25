# 过滤、排序与分页

## 过滤

```rust
let set = ctx.set::<Blog>();

// linq! 过滤（推荐）
let expr = linq!(|b: Blog| b.rating > 3);
let filtered = set.filter(expr).to_list().await?;

// 手动过滤（不使用宏）
let manual = set
    .query()
    .filter_column("rating", ">", 3)
    .to_list()
    .await?;
```

## 排序

```rust
let set = ctx.set::<Blog>();

// linq! 排序
let expr = linq!(|b: Blog| b.active => -b.rating);
let sorted = set.filter(expr).to_list().await?;

// 手动排序
let manual = set
    .query()
    .order_by_desc("rating")
    .to_list()
    .await?;
```

## 分页

```rust
let set = ctx.set::<Blog>();

let page_size = 20;
let page_index = 0;

let page = set
    .query()
    .order_by_desc("created_at")
    .skip(page_index * page_size)
    .take(page_size)
    .to_list()
    .await?;
```

## 组合示例

```rust
let set = ctx.set::<Post>();

let expr = linq!(|p: Post| p.blog_id == target_blog_id);

let posts = set
    .filter(expr)
    .order_by_desc("post_id")
    .skip(0)
    .take(10)
    .to_list()
    .await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 先过滤再分页 | 减少排序和分页的数据量 |
| `skip` + `take` 一起用 | 数据库支持 OFFSET + LIMIT，比内存分页高效 |

下一节：[计数与存在性检查](count-any.md)
