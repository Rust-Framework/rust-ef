# JOIN 查询

当 `include` 不足以表达查询需求时，可用 `linq!` 宏的 `inner_join` / `left_join` 子句手动 JOIN。字符串 API（`inner_join("t", "a", "b")` 等）已移除。

## INNER JOIN

```rust
// 多参数闭包：|a: T1, b: T2| a.col == b.col
let results = linq!(ctx.set::<Blog>();
    inner_join |a: Blog, b: Post| a.blog_id == b.blog_id
).to_list().await?;

// 配合过滤
let rust_posts = linq!(ctx.set::<Blog>();
    inner_join |a: Blog, b: Post| a.blog_id == b.blog_id;
    // 后续可加其他子句
).to_list().await?;
```

## LEFT JOIN

```rust
// 保留左表所有行，右表无匹配则为 NULL
let results = linq!(ctx.set::<Blog>();
    left_join |a: Blog, b: Post| a.blog_id == b.blog_id
).to_list().await?;
```

## JOIN 后的实体物化

手动 JOIN 时，`to_list()` 仍按主实体类型物化。关联数据不会自动填充到导航属性中。如需完整的关系数据，仍推荐使用 `include` 子句：

```rust
// 用 include 自动物化导航（推荐）
let blogs = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;

// 用 join 做跨表过滤（手动物化）
let blogs = linq!(ctx.set::<Blog>();
    inner_join |a: Blog, b: Post| a.blog_id == b.blog_id;
).to_list().await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 优先用 `include` | 关系数据物化自动化，代码更少 |
| JOIN 用于筛选条件 | 当需要根据关联表字段过滤主表时，JOIN 比子查询更直观 |
| 多参数闭包须标注类型 | `|a: Blog, b: Post|` 不能省略类型 |

下一节：[全局查询过滤器](global-query-filters.md)
