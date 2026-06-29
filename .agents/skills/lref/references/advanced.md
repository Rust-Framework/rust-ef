# 第三层：深入理解

> 进阶功能，按需查阅。

## 3.1 linq! 三种形式

**Form A — 过滤闭包（推荐拆成 let）**

```rust
// 可复用表达式 — 推荐写法
let expr = linq!(|b: Blog| b.rating > min_rating);
let set = ctx.set::<Blog>();
let blogs = set.filter(expr).to_list().await?;

// IN 子句
let expr = linq!(|b: Blog| ids.contains(b.id));
let blogs = ctx.set::<Blog>().filter(expr).to_list().await?;

// 直接查询（source + 闭包）— 也可接受
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;
```

**Form B — 多子句查询**

`;` 分隔的子句包括：`include`, `order_by`, `group_by`, `having`, `inner_join`,
`left_join`, `sum`/`avg`/`min`/`max`/`count`, `set` + `execute_update`,
`take`/`skip`, `select` 等。

```rust
// 贪婪加载
linq!(ctx.set::<Blog>(); include b.posts then b.comments).to_list().await?;

// JOIN
linq!(ctx.set::<Post>(); inner_join |p: Post, b: Blog| p.blog_id == b.blog_id)
    .to_list().await?;

// 分组 + HAVING
linq!(ctx.set::<Post>(); group_by b.blog_id; having count(b.post_id) > 1)
    .to_list().await?;

// 聚合
let total: f64 = linq!(ctx.set::<Blog>(); sum b.rating).await?;
```

**Form C — ModelBuilder 配置**

```rust
// 全局查询过滤器
ctx.model().entity::<Blog>()
    .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));

// 索引
ctx.model().entity::<Blog>()
    .has_index(linq!(index |b: Blog| (b.author_id, b.created_at)));
```

## 3.2 批量操作

```rust
// 批量更新
let affected = linq!(
    ctx.set::<Blog>(), |b: Blog| b.rating < 3;
    set b.rating, 3;
    execute_update
).await?;

// 批量删除（直接 DB 删除，不经过跟踪器）
let deleted = linq!(ctx.set::<Post>(), |p: Post| p.blog_id == 0)
    .execute_delete().await?;
```

## 3.3 可复用 LINQ 表达式

```rust
let min_rating = 4;
let expr = linq!(|b: Blog| b.rating > min_rating);

// 同一表达式复用于不同终端
let set = ctx.set::<Blog>();
let blogs = set.filter(expr).to_list().await?;
let count = set.filter(expr).count().await?;
```

## 3.4 SaveChanges 拦截器

```rust
use rust_ef::interceptor::*;

struct AuditInterceptor;
#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        println!("+{} ~{} -{}", ctx.added_count(), ctx.modified_count(), ctx.deleted_count());
        Ok(())
    }
}

// 注册
.add_dbcontext(|options| {
    options
        .use_sqlite("app.db")
        .add_interceptor(AuditInterceptor);
})
```

## 3.5 多数据库（Keyed）

```rust
// 注册
let provider = ServiceCollection::new()
    .add_dbcontext_keyed("primary", |o| o.use_postgres("host=primary/db"))
    .add_dbcontext_keyed("logs", |o| o.use_sqlite("logs.db"))
    .build()
    .unwrap();

// 解析
let primary: Arc<DbContext> = provider.get_keyed("primary");
let logs: Arc<DbContext> = provider.get_keyed("logs");
```

实体通过 `#[context("key")]` 标记归属的数据库上下文。

## 3.6 终端操作速查

| 终端 | 返回类型 | 用途 |
|------|----------|------|
| `.to_list()` | `Vec<T>` | 返回列表 |
| `.first()` | `T` | 首条，无结果报错 |
| `.first_or_default()` | `Option<T>` | 首条或 None |
| `.single()` | `T` | 唯一一条，多条报错 |
| `.single_or_default()` | `Option<T>` | 唯一或 None |
| `.count()` | `i64` | 计数 |
| `.any()` | `bool` | 是否存在 |
| `.all(\|t\| cond)` | `bool` | 是否全部满足 |

## 3.7 架构规则

**应做：**
- 实体相关 trait 以 `I` 为前缀（`IEntityType`, `IDatabaseProvider`）
- 使用 `DbContext`（无需自定义 context 结构体）
- 通过 `add_dbcontext(|o| o.use_sqlite(...))` 注册
- 多数据库使用 `add_dbcontext_keyed("key", |o| ...)`
- 从 DI 解析为 `Arc<DbContext>`

**不应做：**
- 在 context 上定义 `DbSet<Blog>` 结构体字段
- 在 `BelongsTo<T>` 或 `HasMany<T>` 上加 `IEntityType` trait bound
- 在 builder 结构体上加 `IEntityType` bound

> 完整架构文档：`references/architecture.md`