# Eager Loading：Include 与 ThenInclude

`rust-ef` 采用**双查询策略**实现 Eager Loading：先查询主实体，再批量查询关联数据，最后内存物化。这避免了 N+1 问题。

## Include 基础用法

```rust
let blogs = ctx
    .set::<Blog>()
    .query()
    .include_named("posts")
    .to_list()
    .await?;

for blog in &blogs {
    println!("Blog: {}, Posts: {}", blog.url, blog.posts.len());
}
```

## ThenInclude 嵌套加载

```rust
let blogs = ctx
    .set::<Blog>()
    .query()
    .include_named("posts")
    .then_include_named("comments")
    .to_list()
    .await?;

// blog -> posts -> comments 三层结构已物化
```

## 推荐写法

```rust
let set = ctx.set::<Blog>();
let query = set
    .query()
    .include_named("posts")
    .then_include_named("comments");

let blogs = query.to_list().await?;
```

拆分为 `let` 绑定后，每一行的意图都非常清晰。

## 限制

- **无 Lazy Loading**：必须显式调用 `include_named`，否则导航属性为空
- **不支持循环 Include**：如 `Blog -> Post -> Blog` 会导致无限递归，需手动处理

## 设计要点

| 实践 | 说明 |
|------|------|
| 始终预加载需要的导航 | 避免在循环中再次查询数据库 |
| 大数据量分页后再 Include | 先 `take(20)` 再 `include_named`，减少关联查询量 |

下一章：[查询模式](../05-query-patterns/INDEX.md)
