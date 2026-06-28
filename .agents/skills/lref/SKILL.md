---
name: lref
description: |
  Rust Entity Framework (REF) ORM 框架开发指南。涵盖实体定义、linq! 查询、DbContext、
  DI 集成、Web 应用集成、软删除、变更追踪。当用户编写或修改 REF 相关代码时使用。
---

# REF 框架开发指南

Rust Entity Framework — 接口驱动的 EFCore 风格 ORM。本指南按**渐进式披露**组织：
先掌握高频基础，再深入进阶模式，最后了解避坑要点。

---

## 第一层：快速入门

> 90% 的开发场景只需掌握本层内容。深入细节见后续层级。

### 1.1 实体定义

```rust
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(200)]
    pub title: String,

    #[foreign_key(Category)]
    pub category_id: i32,

    #[navigation]
    pub category: BelongsTo<Category>,
}
```

**必知属性速查：**

| 属性 | 用途 | 使用频率 |
|------|------|:---:|
| `#[table("name")]` | 数据库表名 | 必须 |
| `#[primary_key]` | 主键 | 必须 |
| `#[auto_increment]` | 自增主键 | 常用 |
| `#[required]` | NOT NULL | 常用 |
| `#[max_length(N)]` | 字符串最大长度 | 常用 |
| `#[foreign_key(Type)]` | 外键引用 | 常用 |
| `#[navigation]` | 导航属性标记 | 常用 |
| `#[index]` / `#[unique]` | 索引 / 唯一索引 | 常用 |
| `#[column("name")]` | 覆盖列名 | 偶尔 |
| `#[not_mapped]` | 排除映射 | 偶尔 |
| `#[context("key")]` | 多数据库隔离 | 偶尔 |
| `#[concurrency_check]` | 乐观并发令牌 | 罕见 |

> 完整实体定义模板见 `templates/entity-definition.rs`

### 1.2 查询（最常用）

```rust
// 过滤 + 列表
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3)
    .to_list().await?;

// 条件过滤 + 包含导航 + 排序 + 分页
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 0;
    include b.category;
    order_by b.created_at desc;
).skip(0).take(20).to_list().await?;

// 单条查询（主键查）
let blog = ctx.set::<Blog>().query().find(1).await?;

// 首条匹配
let blog = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == "hello")
    .first_or_default().await?;

// 计数
let count = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3; count).await?;
```

### 1.3 增删改

```rust
// 新增
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
// 注意：save_changes() 后 blog.id 已自动填充自增值

// 更新
let mut blog = ctx.set::<Blog>().query().find(1).await?.unwrap();
blog.title = "新标题".into();
ctx.set::<Blog>().detect_changes();  // 仅标记变更字段
ctx.save_changes().await?;

// 删除（软删除推荐用全局查询过滤器，见第二层 §2.4）
let mut blog = ctx.set::<Blog>().query().find(1).await?.unwrap();
blog.is_deleted = true;
ctx.set::<Blog>().detect_changes();
ctx.save_changes().await?;
```

### 1.4 依赖注入

```rust
use rust_dicore::ServiceCollection;
use rust_ef::di::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

let ctx: Arc<dyn IDbContext> = provider.get();
```

> 完整 DI 配置模板见 `templates/di-setup.rs`

---

## 第二层：常用模式

> 生产环境中的高频模式，建议在掌握第一层后阅读。

### 2.1 Web 应用集成

`DbContext` 不是 `Send + Sync`（因为 `save_changes(&mut self)` 需要 `&mut`）。
在 Web 服务器中使用 `Arc<tokio::sync::Mutex<DbContext>>`：

```rust
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Inject)]
pub struct MyHandler {
    ctx: Arc<Mutex<DbContext>>,
}
```

**黄金法则：每次请求只获取一次锁**

在整个写操作期间持有锁，不要在请求内多次获取/释放：

```rust
async fn handle(&self, req: CreateRequest) -> Result<Model> {
    // ✅ 正确：整个写流程持有一把锁
    let mut ctx = self.ctx.lock().await;

    // 1. 唯一性校验（锁内，无 TOCTOU 竞态）
    let exists = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == req.slug)
        .first_or_default().await?;
    if exists.is_some() {
        return Err("Slug 已存在");
    }

    // 2. 插入
    let mut blog = req.to_entity(uid, now);
    ctx.set::<Blog>().add(blog);
    ctx.save_changes().await?;
    // blog.id 已自动填充 —— 无需回查

    // 3. 仅当需要导航属性时，按主键回查
    let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
        include b.category;
    ).first_or_default().await?;

    Ok(saved.to_model())
}
```

**为什么不能多次获取锁？**

| 问题 | 说明 |
|------|------|
| TOCTOU 竞态 | 锁释放后，另一个请求可能在"校验"和"插入"之间插入相同数据 |
| 不必要的开销 | 每次 `lock().await` 都有 Tokio 调度成本 |
| 逻辑复杂化 | 多段锁使代码难以理解和维护 |

> 完整 Web Handler CRUD 模板见 `templates/web-handler-crud.rs`

### 2.2 变更追踪

`save_changes()` 之后需要知道的关键行为：

| 行为 | 说明 |
|------|------|
| 自增 ID 回填 | `save_changes()` 后，实体的 `id` 字段已自动填充数据库生成的值 |
| 跟踪器清空 | 所有已追踪实体被清空，后续查询从数据库重新加载 |
| 导航属性 | 需要导航数据时，按**主键**（不是 slug/email）重新查询并 `include` |

```rust
// 新增后，id 已可用
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
println!("新 ID: {}", blog.id); // ✅ 已填充

// 需要导航属性时，按主键回查
let enriched = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
    include b.category;
).first_or_default().await?;
```

### 2.3 导航属性加载

```rust
// 贪婪加载（推荐）：一次查询加载所有关联数据
linq!(ctx.set::<Blog>(); include b.category; include b.author)
    .to_list().await?;

// 多级加载
linq!(ctx.set::<Blog>(); include b.posts then b.comments)
    .to_list().await?;

// 延迟加载（需在 options 中启用 use_lazy_loading(true)）
let posts = blog.posts.load().await?;
```

### 2.4 软删除（推荐方案）

结合全局查询过滤器 + 实体标记，三步完成：

**步骤 1：定义实体**

```rust
#[derive(Debug, Clone, EntityType)]
#[table("articles")]
pub struct Article {
    #[primary_key] #[auto_increment]
    pub id: i32,
    pub title: String,
    pub is_deleted: bool,  // false = 活跃, true = 已删除
    pub updated_at: i64,
}
```

**步骤 2：启动时注册全局查询过滤器**

```rust
// 在 DbContext 初始化时注册一次
ctx.model().entity::<Article>()
    .has_query_filter(linq!(filter |a: Article| !a.is_deleted));
// 对所有需要软删除的实体重复此操作
```

**步骤 3：执行软删除**

```rust
let mut article = ctx.set::<Article>().query().find(id).await?.unwrap();
article.is_deleted = true;
article.updated_at = now;
ctx.set::<Article>().detect_changes();  // 仅标记变更字段
ctx.save_changes().await?;
```

**管理员查看所有记录（含已删除）：**

```rust
ctx.set::<Article>().query_ignore_filters().to_list().await?;
```

> 完整软删除模板见 `templates/soft-delete.rs`，可运行示例见 `examples/soft_delete/src/main.rs`

### 2.5 查询 API 选择指南

| 场景 | 推荐 API | 示例 |
|------|----------|------|
| 过滤 + 排序 + 导航 | `linq!` Form B | `linq!(ctx.set::<T>(), \|t\| cond; include t.nav; order_by t.f).to_list()` |
| 仅主键查询 | `query().find(id)` | `ctx.set::<T>().query().find(42).await?` |
| 聚合（count/sum/avg） | `linq!` Form A/B | `linq!(ctx.set::<T>(), \|t\| cond; count).await?` |
| 批量更新/删除 | `linq!` execute_update/delete | `linq!(ctx.set::<T>(), \|t\| cond; execute_delete).await?` |
| 忽略全局过滤器 | `query_ignore_filters()` | `ctx.set::<T>().query_ignore_filters().to_list()` |

---

## 第三层：深入理解

> 进阶功能，按需查阅。

### 3.1 linq! 三种形式

**Form A — 过滤闭包**

```rust
// 直接查询
linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 5).to_list().await?;

// 可复用表达式树
let expr = linq!(|b: Blog| b.rating > min_rating);
ctx.set::<Blog>().filter(expr).to_list().await?;

// IN 子句
linq!(ctx.set::<Blog>(), |b: Blog| ids.contains(b.id));
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

### 3.2 批量操作

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

### 3.3 可复用 LINQ 表达式

```rust
let min_rating = 4;
let high_rated = linq!(|b: Blog| b.rating > min_rating);

// 同一表达式复用于不同终端
let blogs = ctx.set::<Blog>().filter(high_rated).to_list().await?;
let count = ctx.set::<Blog>().filter(high_rated).count().await?;
```

### 3.4 SaveChanges 拦截器

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
.add_dbcontext::<DbContext>(|options| {
    options
        .use_sqlite("app.db")
        .add_interceptor(AuditInterceptor);
})
```

### 3.5 多数据库（Keyed）

```rust
// 注册
let provider = ServiceCollection::new()
    .add_dbcontext_keyed::<DbContext>("primary", |o| o.use_postgres("host=primary/db"))
    .add_dbcontext_keyed::<DbContext>("logs", |o| o.use_sqlite("logs.db"))
    .build()
    .unwrap();

// 解析
let primary: Arc<dyn IDbContext> = provider.get_keyed("primary");
let logs: Arc<dyn IDbContext> = provider.get_keyed("logs");
```

实体通过 `#[context("key")]` 标记归属的数据库上下文。

### 3.6 终端操作速查

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

### 3.7 架构规则

**应做：**
- 所有 trait 以 `I` 为前缀（`IDbContext`, `IEntityType`, `IDatabaseProvider`）
- 使用 `DbContext`（无需自定义 context 结构体）
- 通过 `add_dbcontext::<DbContext>(|o| o.use_sqlite(...))` 注册
- 多数据库使用 `add_dbcontext_keyed::<DbContext>("key", |o| ...)`
- 从 DI 解析为 `Arc<dyn IDbContext>`

**不应做：**
- 在 context 上定义 `DbSet<Blog>` 结构体字段
- 在 `BelongsTo<T>` 或 `HasMany<T>` 上加 `IEntityType` trait bound
- 在 builder 结构体上加 `IEntityType` bound

> 完整架构文档见 `references/architecture.md`

---

## 第四层：避坑指南

> 生产环境中已发现的反模式和已知限制，请务必阅读。

### 4.1 save_changes() 后不要回查 ID

`save_changes()` 后自增 ID 已填充到实体上，无需额外查询：

```rust
// ❌ 错误：不必要的数据库往返
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q)
    .first_or_default().await?;
let id = saved.unwrap().id;

// ✅ 正确：直接用实体上的 id
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
let id = blog.id; // 已自动填充！
```

### 4.2 插入后不要按非唯一字段回查

```rust
// ❌ 错误：按 blog_id + user_id 回查并取 max(id) —— 并发场景下不保证取到自己的记录
let created = linq!(ctx.set::<Comment>(), |c: Comment|
    c.blog_id == blog_id && c.user_id == user_id
).to_list().await?;
let last = created.into_iter().max_by_key(|c| c.id).unwrap();

// ✅ 正确：直接用实体上的 id
ctx.set::<Comment>().add(comment);
ctx.save_changes().await?;
let id = comment.id; // 已自动填充
```

### 4.3 不要使用字符串列名 API

```rust
// ❌ 错误：无编译期检查，拼写错误运行时才发现
ctx.set::<Blog>().query()
    .filter_column("slug", "=", "hello")
    .order_by_column("publishd_at")  // 拼写错误！
    .to_list().await?;

// ✅ 正确：linq! 提供编译期类型检查
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == "hello";
    order_by b.published_at desc;
).to_list().await?;
```

### 4.4 不要在每条查询中重复写 is_deleted 过滤

```rust
// ❌ 错误：重复且容易遗漏
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q && !b.is_deleted)
linq!(ctx.set::<User>(), |u: User| !u.is_deleted)
linq!(ctx.set::<Category>(), |c: Category| !c.is_deleted)

// ✅ 正确：启动时注册一次全局查询过滤器
ctx.model().entity::<Blog>()
    .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));
ctx.model().entity::<User>()
    .has_query_filter(linq!(filter |u: User| !u.is_deleted));
// 所有查询自动排除已删除记录
```

### 4.5 不要在写操作中多次获取/释放锁

```rust
// ❌ 错误：三次锁获取，校验和插入之间存在 TOCTOU 竞态
let exists = { let mut ctx = self.ctx.lock().await; ... };
// 锁已释放 —— 另一个请求可以插入相同 slug！
{ let mut ctx = self.ctx.lock().await; ctx.set::<Blog>().add(blog); ... }
let saved = { let mut ctx = self.ctx.lock().await; ... };

// ✅ 正确：整个写流程持有一把锁
let mut ctx = self.ctx.lock().await;
// 校验 → 插入 → 保存 → 回查（如需导航）→ 释放
```

### 4.6 修改操作优先用 detect_changes() 而非 update()

```rust
// ❌ 不够精确：update() 标记整个实体为 Modified
ctx.set::<Blog>().update(blog);
ctx.save_changes().await?;

// ✅ 更好：detect_changes() 仅标记实际变更的字段
blog.is_deleted = true;
blog.updated_at = now;
ctx.set::<Blog>().detect_changes();
ctx.save_changes().await?;
```

### 4.7 已知限制

| 限制 | 说明 | 替代方案 |
|------|------|----------|
| 多自引用外键 | 同一实体多个自引用 FK 时，`linq!` 的 `include` 无法正确区分导航列 | 对第二个 FK 使用裸 `#[foreign_key]`，导航数据手动二次查询 |
| 无 COUNT(DISTINCT) | 框架暂无内置 API | 使用 `group_by` + 内存计数，或通过 provider 执行原始 SQL |
| 无 Form A 的 GROUP BY + 聚合 | 复杂聚合需用 Form B 或内存计算 | 使用 `linq!` Form B 的 `group_by` + `count` 子句 |
| DbContext 不是 Send + Sync | `save_changes(&mut self)` 需要 `&mut` | Web 应用中使用 `Arc<tokio::sync::Mutex<DbContext>>` |

> 更多信息见 `examples/` 目录下的可运行示例。