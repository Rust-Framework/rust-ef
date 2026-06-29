# 第二层：rust-webapp 集成

> rust-webapp 就相当于 ASP.NET Core。本节覆盖从 DbContext 注册到 Handler 实现的全部推荐写法，是生产环境的核心参考。

## 2.1 上下文注册（对标 AddDbContext）

在 `main.rs` 的 `Host::builder()` 中使用 `add_dbcontext`，框架按 **Scoped** 生命周期管理。每个请求获得独立的 `DbContext` 实例，天然隔离，无需锁。
> **rust-webapp 自动管理 Scope**：HTTP 管道为每个请求自动创建 DI Scope，Handler 通过 **owned 解析**（`get_owned()`）获得专属 `DbContext` 实例，**无需手动 `create_scope()`**。只有非请求场景（如 `IHostedService` 启动任务）才需要手动创建 Scope。
```rust
// main.rs — 组合根
use rust_webapp::*;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::main]
async fn main() {
    let host = Host::builder()
        .register(|svc| register_db_context(svc))
        .add_options::<SiteConfig>("Site")
        .build();

    host.run().await.expect("Server failed");
}

/// 注册 DbContext — 类似 ASP.NET Core 的 AddDbContext<AppDbContext>()
fn register_db_context(svc: ServiceCollection) -> ServiceCollection {
    svc.add_dbcontext(|options| {
        match AppMode::from_env() {
            AppMode::Production => {
                let cs = std::env::var("DATABASE_URL").unwrap();
                options.use_mysql(&cs);
            }
            AppMode::Development => {
                let path = app_base().join("app.db");
                options.use_sqlite(&path.to_string_lossy());
            }
        }
    })
}
```

**关键点：**
- `add_dbcontext` 注册为 **Scoped**，不是 Singleton
- 生产和开发环境自动切换数据库
- Handler 通过 owned 解析获得 `DbContext`（`&mut self` 访问），无需 `Arc<Mutex>`

**启动时初始化（种子数据 + 建表 + 全局查询过滤器）：**

```rust
// startup.rs — 实现 IHostedService，在 host 启动时执行
use rust_ef::db_context::DbContext;

#[derive(Inject)]
pub struct DbInitService {
    provider: Arc<ServiceProvider>,
}

#[inject(scoped)]
#[async_trait]
impl IHostedService for DbInitService {
    async fn start(&self) -> Result<()> {
        // Owned 解析：获得专有 DbContext，直接 &mut self 访问
        let mut ctx: DbContext = self.provider.get_owned();

        // 注册种子数据到 model builder
        seed(&mut ctx);

        // 建表 + 应用种子数据
        ctx.ensure_created().await?;

        // 注册全局查询过滤器（软删除）
        ctx.model().entity::<Blog>()
            .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));
        ctx.model().entity::<User>()
            .has_query_filter(linq!(filter |u: User| !u.is_deleted));
        // ... 对所有需要软删除的实体重复
        Ok(())
    }
}
```

> **注意**：全局查询过滤器必须在 `set::<T>()` 之前注册。`DbSet` 创建时从 `ModelBuilder` 读取过滤器并缓存。

## 2.2 Handler 注入模式（≈ 构造函数注入）

每个 Handler 是一个独立的 struct，通过 `#[derive(Inject)]` 声明依赖。`ctx: DbContext` 字段（bare T）由 DI 容器通过 **owned 解析**自动注入——类似 ASP.NET Core 的构造函数注入。
> **无需管理 Scope**：rust-webapp 的 HTTP 管道为每个请求创建 Scope，Handler 在此 Scope 内通过 `get_owned::<Handler>()` 解析，每个请求获得独立 `DbContext` 实例，天然隔离，无需锁。
>
> **Owned 解析 + `&mut self`**：`#[derive(Inject)]` 自动检测 bare `T` 字段并使用 `get_owned()` 解析。Handler 方法使用 `&mut self`，直接调用 `self.ctx.set::<T>()` / `self.ctx.save_changes()` —— 无需 `Arc<Mutex>`，无需内部可变性。
>
> **`#[inject(scoped)]` 必须**：`#[inject]` 默认注册为 Singleton，与 Scoped `DbContext` 形成 captive dependency。Handler 的 trait impl 必须使用 `#[inject(scoped)]`。

**Handler 定义：**

```rust
// 每个操作一个 Handler struct（单一职责）
#[derive(Inject)]
pub struct ListBlogPostsHandler {
    ctx: DbContext,  // bare T → owned 解析
}

#[derive(Inject)]
pub struct GetBlogPostHandler {
    ctx: DbContext,
}

#[derive(Inject)]
pub struct CreateBlogPostHandler {
    ctx: DbContext,
}

#[derive(Inject)]
pub struct UpdateBlogPostHandler {
    ctx: DbContext,
}

#[derive(Inject)]
pub struct DeleteBlogPostHandler {
    ctx: DbContext,
}
```

**路由与请求绑定（在 contracts crate 中）：**

```rust
// contracts/blog.rs — 契约层，定义路由、DTO、接口
use rust_webapp::*;

#[derive(Deserialize)]
pub struct ListBlogPostsRequest;

#[get("/api/blog")]
impl IRequest<Vec<BlogPostSummary>> for ListBlogPostsRequest {}

#[derive(Deserialize)]
pub struct GetBlogPostRequest {
    #[param(path)]
    pub slug: String,
}

#[get("/api/blog/{slug}")]
impl IRequest<BlogPostModel> for GetBlogPostRequest {}

#[derive(Deserialize)]
pub struct CreateBlogPostRequest {
    pub title: String,
    pub slug: String,
    pub content: String,
    pub category_id: i32,
    pub tags: Option<Vec<String>>,
    #[claims]
    pub claims: Option<Arc<dyn IClaims>>,  // 框架自动注入认证信息
}

#[post("/api/blog")]
#[authorize]
impl IRequest<BlogPostModel> for CreateBlogPostRequest {}
```

## 2.3 读取操作（Read）

**列表查询（分页 + 导航 + 排序）：**

```rust
#[inject(scoped)]
#[async_trait]
impl IRequestHandler<ListBlogPostsRequest, Vec<BlogPostSummary>> for ListBlogPostsHandler {
    async fn handle(&mut self, _: ListBlogPostsRequest) -> Result<Vec<BlogPostSummary>> {
        let blogs = linq!(self.ctx.set::<Blog>(), |b: Blog| !b.is_deleted;
            include b.category;
            include b.author;
            order_by b.published_at desc;
        ).skip(0).take(20).to_list().await?;
        Ok(blogs.into_iter().map(BlogPostSummary::from).collect())
    }
}
```

**单条查询（按 slug / id）：**

```rust
#[inject(scoped)]
#[async_trait]
impl IRequestHandler<GetBlogPostRequest, BlogPostModel> for GetBlogPostHandler {
    async fn handle(&mut self, req: GetBlogPostRequest) -> Result<BlogPostModel> {
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let blog = linq!(set, expr;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or_else(|| Error::NotFound(format!("Blog not found: {}", req.slug)))?;
        Ok(blog.to_model())
    }
}
```

**按认证用户过滤（claims 注入）：**

```rust
#[inject(scoped)]
#[async_trait]
impl IRequestHandler<ListMyBlogPostsRequest, Vec<BlogPostSummary>> for ListMyBlogPostsHandler {
    async fn handle(&mut self, req: ListMyBlogPostsRequest) -> Result<Vec<BlogPostSummary>> {
        let uid = uid_from_claims(req.claims.as_deref())?;
        let blogs = linq!(self.ctx.set::<Blog>(), |b: Blog| b.author_id == uid;
            include b.category;
            order_by b.published_at desc;
        ).to_list().await?;
        Ok(blogs.into_iter().map(BlogPostSummary::from).collect())
    }
}
```

## 2.4 创建操作（Create）

```rust
#[inject(scoped)]
#[async_trait]
impl IRequestHandler<CreateBlogPostRequest, BlogPostModel> for CreateBlogPostHandler {
    async fn handle(&mut self, req: CreateBlogPostRequest) -> Result<BlogPostModel> {
        // 1. 从 claims 提取用户 ID
        let uid = req.claims.as_ref()
            .and_then(|c| c.subject().parse::<i32>().ok())
            .ok_or_else(|| Error::Http("Not authenticated".into()))?;

        // 2. 唯一性校验
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let exists = set.filter(expr).first_or_default().await?;
        if exists.is_some() {
            return Err(Error::Http("Slug already exists".into()));
        }

        // 3. 构造实体并插入
        let now = chrono::Utc::now().timestamp();
        let mut blog = req.to_entity(uid, now);
        self.ctx.set::<Blog>().add(blog);
        self.ctx.save_changes().await?;
        // blog.id 已自动填充——无需回查

        // 4. 仅当需要导航属性时，按主键回查
        let saved = linq!(self.ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or_else(|| Error::Internal("Blog vanished after insert".into()))?;

        tracing::info!("[Blog] Created: {} by {}", saved.slug, uid);
        Ok(saved.to_model())
    }
}
```

## 2.5 更新操作（Update）

```rust
#[inject(scoped)]
#[async_trait]
impl IRequestHandler<UpdateBlogPostRequest, BlogPostModel> for UpdateBlogPostHandler {
    async fn handle(&mut self, req: UpdateBlogPostRequest) -> Result<BlogPostModel> {
        // 1. 加载现有实体
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let mut blog = set.filter(expr).first_or_default().await?
            .ok_or_else(|| Error::NotFound(format!("Blog not found: {}", req.slug)))?;

        // 2. 权限校验：非管理员只能修改自己的文章
        let uid = uid_from_claims(req.claims.as_deref())?;
        let roles = roles_from_claims(req.claims.as_deref());
        if !is_admin(&roles) && blog.author_id != uid {
            return Err(Error::Http("Forbidden: not the author".into()));
        }

        // 3. 应用变更
        let now = chrono::Utc::now().timestamp();
        req.apply_to(&mut blog, uid, now);

        // 4. 保存（detect_changes 仅标记实际变更的字段）
        self.ctx.set::<Blog>().detect_changes();
        self.ctx.save_changes().await?;

        // 5. 回查导航属性（按主键）
        let saved = linq!(self.ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or_else(|| Error::NotFound("Blog not found after update".into()))?;

        Ok(saved.to_model())
    }
}
```

## 2.6 删除操作（Delete — 软删除）

```rust
#[inject(scoped)]
#[async_trait]
impl IRequestHandler<DeleteBlogPostRequest, String> for DeleteBlogPostHandler {
    async fn handle(&mut self, req: DeleteBlogPostRequest) -> Result<String> {
        // 1. 加载实体
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let mut blog = set.filter(expr).first_or_default().await?
            .ok_or_else(|| Error::NotFound(format!("Blog not found: {}", req.slug)))?;

        // 2. 权限校验
        let uid = uid_from_claims(req.claims.as_deref())?;
        let roles = roles_from_claims(req.claims.as_deref());
        if !is_admin(&roles) && blog.author_id != uid {
            return Err(Error::Http("Forbidden: not the author".into()));
        }

        // 3. 软删除（标记 + detect_changes + 保存）
        blog.is_deleted = true;
        blog.updated_at = chrono::Utc::now().timestamp();
        blog.updated_id = Some(uid);
        self.ctx.set::<Blog>().detect_changes();
        self.ctx.save_changes().await?;

        Ok(format!("Deleted blog {}", req.slug))
    }
}
```

> **前提**：启动时已注册全局查询过滤器 `has_query_filter(linq!(filter |b: Blog| !b.is_deleted))`。标记 `is_deleted = true` 后该记录自动从所有查询中排除。

## 2.7 错误处理

统一使用 `rust_webapp::Error` 类型，按场景映射。
| 场景 | 错误类型 | 示例 |
|------|----------|------|
| 资源不存在 | `Error::NotFound` | `Error::NotFound(format!("Blog not found: {}", slug))` |
| 业务校验失败 | `Error::Http` | `Error::Http("Slug already exists".into())` |
| 权限不足 | `Error::Http` | `Error::Http("Forbidden: not the author".into())` |
| 数据库异常 | `Error::Internal` | `Error::Internal(format!("DB error: {}", e))` |
| 参数校验失败 | `Error::Validation` | `Error::Validation("Title is required".into())` |

**辅助函数（提取 claims 信息）：**

```rust
fn uid_from_claims(claims: Option<&dyn IClaims>) -> Result<i32> {
    let c = claims.ok_or_else(|| Error::Http("Not authenticated".into()))?;
    c.subject()
        .parse::<i32>()
        .map_err(|_| Error::Http("Invalid user id in token".into()))
}

fn roles_from_claims(claims: Option<&dyn IClaims>) -> Vec<String> {
    claims.map(|c| c.roles().to_vec()).unwrap_or_default()
}

fn is_admin(roles: &[String]) -> bool {
    roles.iter().any(|r| r == "admin")
}
```

## 2.8 服务层模式（可选）

当业务逻辑复杂、需要跨 Handler 复用时，引入 Service 抽象。

```rust
// contracts/blog.rs — 契约层
pub trait IBlogService: Send + Sync {
    fn list_posts(&self) -> Result<Vec<BlogPostSummary>, String>;
    fn create_post(&self, req: CreateBlogPostRequest) -> Result<BlogPostModel, String>;
}

// handlers/blog_service.rs — 实现层
#[derive(Inject)]
pub struct BlogService {
    ctx: DbContext,  // bare T → owned 解析
}

impl IBlogService for BlogService {
    fn list_posts(&self) -> Result<Vec<BlogPostSummary>, String> {
        todo!()
    }
}

// handlers/blog_handler.rs — 调用 Handler
#[derive(Inject)]
pub struct CreateBlogPostHandler {
    blog: Arc<dyn IBlogService>,  // 注入服务，而非直接注入 DbContext
}

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<CreateBlogPostRequest, BlogPostModel> for CreateBlogPostHandler {
    async fn handle(&mut self, req: CreateBlogPostRequest) -> Result<BlogPostModel> {
        self.blog.create_post(req)
            .map_err(|e| Error::Internal(e))
    }
}
```

**何时使用 Service 层：**

| 场景 | 建议 |
|------|------|
| 简单 CRUD，无需 Handler 复用 | 直接注入 `DbContext`（owned） |
| 复杂业务逻辑，多 Handler 共享 | 抽取 `I...Service`，注入 `Arc<dyn I...Service>` |
| 需要 mock 测试 | 引入 Service 接口便于替换实现 |

## 2.9 变更追踪

`save_changes()` 之后需要知道的关键行为。
| 行为 | 说明 |
|------|------|
| 自增 ID 回填 | `save_changes()` 后，实体的 `id` 字段已自动填充数据库生成的值 |
| 跟踪器清空 | 所有已追踪实体被清空，后续查询从数据库重新加载 |
| 导航属性 | 需要导航数据时，按**主键**（不是 slug/email）重新查询并 `include` |

```rust
// 新增后，id 已可用
ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
println!("新 ID: {}", blog.id); // 已填充
// 需要导航属性时，按主键回查
let enriched = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
    include b.category;
).first_or_default().await?;
```

## 2.10 导航属性加载

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

## 2.11 软删除（在 REF 层面）

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
// 在 DbInitService::start() 中注册一次
ctx.model().entity::<Article>()
    .has_query_filter(linq!(filter |a: Article| !a.is_deleted));
// 对所有需要软删除的实体重复此操作
```

**步骤 3：执行软删除**

```rust
let query = ctx.set::<Article>().query();
let mut article = query.find(id).await?.unwrap();
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

## 2.12 查询 API 选择指南

| 场景 | 推荐 API | 示例 |
|------|----------|------|
| 过滤 + 排序 + 导航 | `linq!` Form B | `linq!(ctx.set::<T>(), \|t\| cond; include t.nav; order_by t.f).to_list()` |
| 仅主键查询 | `query().find(id)` | `let query = ctx.set::<T>().query(); query.find(42).await?` |
| 聚合（count/sum/avg） | `linq!` Form B | `linq!(ctx.set::<T>(), \|t\| cond; count).await?` |
| 批量更新/删除 | `linq!` execute_update/delete | `linq!(ctx.set::<T>(), \|t\| cond; execute_delete).await?` |
| 忽略全局过滤器 | `query_ignore_filters()` | `ctx.set::<T>().query_ignore_filters().to_list()` |