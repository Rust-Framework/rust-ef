# 多租户基础设施

rust-ef 提供多租户数据隔离的能力（而非写死实现），核心机制是**全局查询过滤器** + **Scoped 生命周期**。

## 1. 线程安全契约

`DbContext` **非线程安全**——单个实例禁止跨线程共享。这是设计决策（对齐 EFCore），不是限制。

### 正确用法：Owned 解析（推荐）

每个请求/操作通过 `get_owned()` 获得专属 `DbContext` 实例，直接 `&mut self` 访问，无需锁：

```rust,ignore
let provider = ServiceCollection::new()
    .add_dbcontext(|o| o.use_sqlite("app.db"))
    .build()?;

// 推荐：owned 解析，直接 &mut self 访问
let mut ctx: DbContext = provider.get_owned();
```

### 共享解析（同一 Scope 内复用）

当多个消费者在同一 Scope 内需要共享同一实例时，使用 `scope.get()` 获取 `Arc<DbContext>`（仅 `&self` 访问）：

```rust,ignore
let scope = provider.create_scope();
let ctx: Arc<DbContext> = scope.get();
// 同一 scope 内多次 get 返回同一实例
```

### 反模式：Arc<Mutex<DbContext>>

```rust,ignore
// ❌ 错误：共享会导致跟踪污染
let ctx = Arc::new(Mutex::new(ctx));
// Thread A 的 save_changes() 会提交 Thread B 挂起的变更
```

### 从根 ServiceProvider 直接解析

从根 `ServiceProvider` 直接 `get::<DbContext>()` 会在 root scope 缓存中复用同一实例（rust-dix 0.6+ 行为）。使用 `get_owned::<DbContext>()` 则每次返回新实例。

## 2. 多租户查询过滤器

通过 `ModelBuilder.has_query_filter` 注册租户隔离谓词，框架自动应用到多种 DML 操作。

### 注册过滤器

```rust
let mut ctx = DbContext::from_options(&options)?;
// from_options() 已自动调用 discover_entities()，无需手动注册

// 注册租户隔离过滤器（tenant_id = 1）
ctx.model().has_query_filter::<Blog>(
    linq!(filter |b: Blog| b.tenant_id == 1)
);

ctx.set::<Blog>();
ctx.ensure_created().await?;
```

> **注意**：过滤器必须在 `set::<T>()` 之前注册。`DbSet` 创建时从 `ModelBuilder` 读取过滤器并缓存。

### 覆盖范围

| 操作 | 是否自动过滤 | 说明 |
|------|:---:|------|
| SELECT | 是 | `query().to_list()` 自动追加 WHERE |
| UPDATE | 是 | `save_changes()` 的 UPDATE 语句 AND 过滤器 |
| DELETE | 是 | `save_changes()` 的 DELETE 语句 AND 过滤器 |
| Navigation (Include) | 是 | 二级查询 SELECT 自动追加 WHERE |
| INSERT | 否 | 用户在 `add()` 前手动设置 `tenant_id` |

### INSERT 不自动过滤的原因

INSERT 时框架无法知道当前租户 ID（过滤器表达式中的值是注册时绑定的，而非每次操作动态传入）。用户需在 `add()` 前显式设置：

```rust
let blog = Blog {
    tenant_id: current_tenant_id,  // 显式设置
    title: "My Blog".into(),
    ..
};
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
```

这体现了"框架提供能力，不写死实现"的原则——用户可以自由选择租户 ID 的来源（JWT claim、请求头、配置等）。

### Navigation 自动过滤示例

```rust
// Blog 有 HasMany<Post>，Post 有 tenant_id 过滤器
// 加载 Blog 时 Include Posts，只返回同租户的 Posts
let blogs = linq!(ctx.set::<Blog>(); include b.posts)
    .to_list()
    .await?;
// blogs[0].posts 只含 tenant_id 匹配的 Posts
```

Navigation 过滤器由 `ModelBuilder.filters_by_table()` 收集所有实体的过滤器，按表名索引，通过 `Arc<HashMap<String, BoolExpr>>` 共享给 `QueryBuilder`，最终传递给 `NavigationLoader`。

## 3. 跨租户管理查询

管理后台需要查看所有租户数据时，使用 `query_ignore_filters()` 绕过查询过滤器：

```rust
// 返回所有租户的数据
let all_blogs = ctx.set::<Blog>()
    .query_ignore_filters()
    .to_list()
    .await?;
```

> **语义**：`query_ignore_filters()` 只绕过**当前实体**的查询过滤器。Navigation 加载的**关联实体**过滤器仍生效（对应 EFCore `IgnoreQueryFilters` 语义）。

## 4. 完整示例

```rust
use rust_ef::prelude::*;
use rust_ef::linq;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
struct Blog {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    tenant_id: i32,
    #[required]
    title: String,
    #[navigation]
    posts: HasMany<Post>,
}

#[derive(Debug, Clone, EntityType)]
#[table("posts")]
struct Post {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    tenant_id: i32,
    #[required]
    title: String,
    #[foreign_key(Blog)]
    blog_id: i32,
}

async fn run() -> EFResult<()> {
    let mut ctx = DbContext::from_options(&options)?;

    // 注册租户过滤器（必须在 set::<T>() 之前）
    ctx.model()
        .has_query_filter::<Blog>(linq!(filter |b: Blog| b.tenant_id == 1))
        .has_query_filter::<Post>(linq!(filter |p: Post| p.tenant_id == 1));

    ctx.set::<Blog>();
    ctx.set::<Post>();
    ctx.ensure_created().await?;

    // INSERT：手动设置 tenant_id
    ctx.set::<Blog>().add(Blog {
        id: 0,
        tenant_id: 1,
        title: "Tenant 1 Blog".into(),
        posts: HasMany::new(),
    });
    ctx.save_changes().await?;

    // SELECT：自动过滤 tenant_id=1
    let blogs = ctx.set::<Blog>().query().to_list().await?;
    // Navigation：Posts 自动过滤 tenant_id=1
    let blogs_with_posts = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;

    // 管理查询：绕过过滤器
    let all = ctx.set::<Blog>().query_ignore_filters().to_list().await?;

    Ok(())
}
```

## 5. 反模式警示

| 反模式 | 后果 | 正确做法 |
|--------|------|----------|
| `Arc<Mutex<DbContext>>` 共享 | 跟踪污染，跨请求提交 | 用 owned 解析或 DI Scope 隔离 |
| 过滤器在 `set::<T>()` 后注册 | DbSet 未注入过滤器 | 先注册过滤器再 `set::<T>()` |
| 依赖 INSERT 自动设置 tenant_id | 框架无法知道运行时租户 | `add()` 前手动设置 |
| 用 `query_ignore_filters()` 做权限控制 | 绕过安全边界 | 权限校验放 Service 层 |

## 相关文档

- [全局查询过滤器（基础用法）](../06-advanced-query/global-query-filters.md)
- [DI 注册与 Scoped 生命周期](../10-di-interceptors/di-registration.md)
