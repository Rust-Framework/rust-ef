# Lazy Loading：按需加载（v1.1）

Lazy Loading 允许在首次访问导航属性时自动从数据库加载关联数据，无需在查询时显式 `include`。v1.1.0 起支持，**默认关闭**（opt-in），以保留 v1.0 的 eager-only 行为。

## 启用 Lazy Loading

```rust
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let mut builder = DbContextOptionsBuilder::new();
builder.use_sqlite("app.db");
builder.use_lazy_loading(true);           // ← opt-in
let mut ctx = DbContext::from_options(&builder.build())?;
```

## 基本用法

启用后，`to_list()` 返回的实体导航属性会挂载延迟加载上下文（`LazyContext`）。首次调用 `load()` 时触发单实体查询，后续访问读取内存缓存：

```rust
let mut blogs = ctx.set::<Blog>().query().to_list().await?;

for blog in blogs.iter_mut() {
    if !blog.posts.is_loaded() {
        blog.posts.load().await?;          // 首次访问触发 SQL 查询
    }
    println!("{}: {} posts", blog.url, blog.posts.len());
}
```

### BelongsTo 延迟加载

```rust
let mut posts = ctx.set::<Post>().query().to_list().await?;

for post in posts.iter_mut() {
    post.blog.load().await?;               // 加载关联的 Blog
    let blog = post.blog.item();
    println!("Post {} -> Blog {}", post.title, blog.url);
}
```

## API 速查

| 方法 | 适用容器 | 说明 |
|------|---------|------|
| `is_loaded()` | `HasMany` / `HasOne` / `BelongsTo` | 返回导航是否已加载 |
| `load().await` | `HasMany` / `HasOne` / `BelongsTo` | 触发延迟加载；已加载时为 no-op |
| `items()` | `HasMany` | 返回 `&[T]`，已加载的子实体切片 |
| `len()` | `HasMany` | 返回已加载子实体数量 |
| `item()` | `BelongsTo` / `HasOne` | 返回 `&T`，已加载的关联实体 |

## Include 优先级

`include` 子句（Eager Loading）**优先于** Lazy Loading。如果查询已通过 `include` 预加载了导航属性，`is_loaded()` 直接返回 `true`，`load()` 为 no-op：

```rust
// Eager Loading + Lazy Loading 同时启用时，include 优先
let blogs = linq!(ctx.set::<Blog>(); include b.posts)
    .to_list()
    .await?;

assert!(blogs[0].posts.is_loaded());       // ✅ 已通过 include 加载
```

## 递归深度保护

Lazy Loading 内置 `MAX_LAZY_DEPTH = 16` 递归深度保护，防止无限递归加载（如 `Blog -> Post -> Blog -> ...`）。超过深度限制时返回错误。

## 未启用时的行为

未开启 `use_lazy_loading(true)` 时：
- 导航属性为空（`is_loaded()` 返回 `false`，`len()` 返回 `0`）
- 调用 `load()` 是**安全 no-op**（不报错，但不加载任何数据）
- 必须使用 `linq!(...; include ...)` 进行 Eager Loading

```rust
// Lazy Loading 未开启时
let blogs = ctx.set::<Blog>().query().to_list().await?;
assert!(!blogs[0].posts.is_loaded());
blogs[0].posts.load().await?;              // safe no-op，不加载
assert!(!blogs[0].posts.is_loaded());      // 仍然 false
```

## 设计要点

| 实践 | 说明 |
|------|------|
| **默认关闭** | 保留 v1.0 eager-only 行为，避免意外的 N+1 查询 |
| **按需开启** | 仅在确实需要延迟加载的场景启用 |
| **Include 优先** | 已知需要关联数据时，始终用 `include` 预加载更高效 |
| **避免循环中 load()** | 循环内逐条 `load()` 会产生 N+1 查询，应改用 `include` |
| **幂等加载** | `load()` 可安全重复调用，第二次为 no-op |

## Eager vs Lazy 选择指南

| 场景 | 推荐 | 原因 |
|------|------|------|
| 列表页需要显示关联数据 | Eager (`include`) | 一次性批量加载，避免 N+1 |
| 详情页按需展开子数据 | Lazy | 用户可能不展开所有子项 |
| 条件性访问导航 | Lazy | 仅在满足条件时触发加载 |
| 固定结构的嵌套数据 | Eager (`include ... then ...`) | 已知结构，预加载更高效 |

## 限制

- **`#[derive(EntityType)]` 自动生成 `ILazyInit`**：所有实体自动支持 Lazy Loading，无需手动实现
- **DbContext 必须存活**：`load()` 依赖 `DbContext` 的连接，DbContext 被销毁后无法加载
- **`from_row` 限制**：Window 函数投影列被 `from_row` 忽略，Lazy Loading 不受影响

上一节：[Eager Loading：Include 与 ThenInclude](eager-loading.md)

下一章：[查询模式](../05-query-patterns/INDEX.md)
