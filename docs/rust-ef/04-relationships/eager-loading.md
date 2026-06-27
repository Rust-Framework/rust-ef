# Eager Loading：Include 与 ThenInclude

`rust-ef` 采用**双查询策略**实现 Eager Loading：先查询主实体，再批量查询关联数据，最后内存物化。这避免了 N+1 问题。

Eager Loading 通过 `linq!` 宏的 `include` 子句表达，导航字段在闭包中以 `b.posts` 形式访问（编译期解析为 `Blog::FIELD_POSTS` 常量）。

## Include 基础用法

```rust
let blogs = linq!(ctx.set::<Blog>(); include b.posts)
    .to_list()
    .await?;

for blog in &blogs {
    println!("Blog: {}, Posts: {}", blog.url, blog.posts.len());
}
```

## ThenInclude 嵌套加载

```rust
let blogs = linq!(ctx.set::<Blog>();
    include b.posts then b.comments
)
.to_list()
.await?;

// blog -> posts -> comments 三层结构已物化
```

`then` 链可以多层嵌套：`include b.posts then b.comments then b.author`。

## 推荐写法

```rust
// 含过滤与排序的完整查询
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.published;
    include b.posts then b.comments;
    order_by b.created_at desc;
).to_list().await?;
```

## 限制

- **Lazy Loading 默认关闭**：不开启时必须显式 `include`，否则导航属性为空。v1.1.0 起可通过 `use_lazy_loading(true)` 开启按需加载（详见 [Lazy Loading](../04-relationships/lazy-loading.md)）
- **不支持循环 Include**：如 `Blog -> Post -> Blog` 会导致无限递归，需手动处理

## 设计要点

| 实践 | 说明 |
|------|------|
| 始终预加载需要的导航 | 避免在循环中再次查询数据库 |
| 大数据量分页后再 Include | 先 `take 20` 再 `include`，减少关联查询量 |

下一章：[查询模式](../05-query-patterns/INDEX.md)
