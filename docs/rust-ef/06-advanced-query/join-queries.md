# JOIN 查询

当 `Include` 不足以表达查询需求时，可手动使用 JOIN。

## INNER JOIN

```rust
let results = ctx
    .set::<Post>()
    .query()
    .inner_join("blogs", "blog_id", "blog_id")
    .filter(linq!(|p: Post| p.title.contains("Rust")))
    .to_list()
    .await?;
```

## LEFT JOIN

```rust
let results = ctx
    .set::<Blog>()
    .query()
    .left_join("posts", "blog_id", "blog_id")
    .to_list()
    .await?;
```

## JOIN 后的实体物化

手动 JOIN 时，`to_list()` 仍按主实体类型物化。关联数据不会自动填充到导航属性中。如需完整的关系数据，仍推荐使用 `include_named`。

## 设计要点

| 实践 | 说明 |
|------|------|
| 优先用 `Include` / `ThenInclude` | 关系数据物化自动化，代码更少 |
| JOIN 用于筛选条件 | 当需要根据关联表字段过滤主表时，JOIN 比子查询更直观 |

下一节：[全局查询过滤器](global-query-filters.md)
