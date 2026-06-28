# 第二层：rust-webapp 集成

> rust-webapp 就相当于 ASP.NET Core。本节覆盖从 DbContext 注册�?Handler 实现�?> 全部推荐写法，是生产环境的核心参考�?
## 2.1 上下文注册（�?AddDbContext�?
�?`main.rs` �?`Host::builder()` 中使�?`add_dbcontext`，框架按 **Scoped** 生命周期管理�?每个请求获得独立�?`DbContext` 实例，天然隔离，无需锁�?
> **rust-webapp 自动管理 Scope**：HTTP 管道为每个请求自动创�?DI Scope�?> Handler 中的 `ctx: Arc<dyn IDbContext>` 由框架自动解析注入，**无需手动 `create_scope()`**�?> 只有非请求场景（�?`IHostedService` 启动任务）才需要手动创�?Scope�?
```rust
// main.rs —�?组合�?use rust_webapp::*;
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

/// 注册 DbContext —�?类似 ASP.NET Core �?AddDbContext<AppDbContext>()
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
- `add_dbcontext` 注册�?**Scoped**，不�?Singleton
- 生产和开发环境自动切换数据库
- 解析�?`Arc<dyn IDbContext>`，支�?trait object 跨层传�?
**启动时初始化（种子数�?+ 建表 + 全局查询过滤器）�?*

```rust
// startup.rs —�?实现 IHostedService，在 host 启动时执�?use rust_ef::db_context::IDbContext;
// rust-webapp 自动管理 Scope，Handler 无需手动创建

#[derive(Inject)]
pub struct DbInitService {
    provider: Arc<ServiceProvider>,
}

#[inject]
#[async_trait]
impl IHostedService for DbInitService {
    async fn start(&self) -> Result<()> {
        // 创建独立 Scope，获得专�?DbContext
        let scope = self.provider.create_scope();
        let ctx: Arc<dyn IDbContext> = scope.get();

        // 注册种子数据�?model builder
        seed(&mut ctx);

        // 建表 + 应用种子数据
        ctx.ensure_created().await?;

        // 注册全局查询过滤器（软删除）
        ctx.model().entity::<Blog>()
            .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));
        ctx.model().entity::<User>()
            .has_query_filter(linq!(filter |u: User| !u.is_deleted));
        // ... 对所有需要软删除的实体重�?
        Ok(())
    }
}
```

> **注意**：全局查询过滤器必须在 `set::<T>()` 之前注册。`DbSet` 创建时从 `ModelBuilder` 读取过滤器并缓存�?
## 2.2 Handler 注入模式（≈ 构造函数注入）

每个 Handler 是一个独立的 struct，通过 `#[derive(Inject)]` 声明依赖�?`ctx: Arc<dyn IDbContext>` 字段�?DI 容器自动解析——类�?ASP.NET Core 的构造函数注入�?
> **无需管理 Scope**：rust-webapp �?HTTP 管道已为每个请求创建 Scope�?> Handler 只需声明 `ctx: Arc<dyn IDbContext>`，框架自动注入对应的实例�?
**Handler 定义�?*

```rust
// 每个操作一�?Handler struct（单一职责�?#[derive(Inject)]
pub struct ListBlogPostsHandler {
    ctx: Arc<dyn IDbContext>,
}

#[derive(Inject)]
pub struct GetBlogPostHandler {
    ctx: Arc<dyn IDbContext>,
}

#[derive(Inject)]
pub struct CreateBlogPostHandler {
    ctx: Arc<dyn IDbContext>,
}

#[derive(Inject)]
pub struct UpdateBlogPostHandler {
    ctx: Arc<dyn IDbContext>,
}

#[derive(Inject)]
pub struct DeleteBlogPostHandler {
    ctx: Arc<dyn IDbContext>,
}
```

**路由与请求绑定（�?contracts crate 中）�?*

```rust
// contracts/blog.rs —�?契约层，定义路由、DTO、接�?use rust_webapp::*;

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

## 2.3 读取操作（Read�?
**列表查询（分�?+ 导航 + 排序）：**

```rust
#[inject]
#[async_trait]
impl IRequestHandler<ListBlogPostsRequest, Vec<BlogPostSummary>> for ListBlogPostsHandler {
    async fn handle(&self, _: ListBlogPostsRequest) -> Result<Vec<BlogPostSummary>> {
        let blogs = linq!(ctx.set::<Blog>(), |b: Blog| !b.is_deleted;
            include b.category;
            include b.author;
            order_by b.published_at desc;
        ).skip(0).take(20).to_list().await?;
        Ok(blogs.into_iter().map(BlogPostSummary::from).collect())
    }
}
```

**单条查询（按 slug / �?id）：**

```rust
#[inject]
#[async_trait]
impl IRequestHandler<GetBlogPostRequest, BlogPostModel> for GetBlogPostHandler {
    async fn handle(&self, req: GetBlogPostRequest) -> Result<BlogPostModel> {
        let set = ctx.set::<Blog>();
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

**按认证用户过滤（`claims` 注入）：**

```rust
#[inject]
#[async_trait]
impl IRequestHandler<ListMyBlogPostsRequest, Vec<BlogPostSummary>> for ListMyBlogPostsHandler {
    async fn handle(&self, req: ListMyBlogPostsRequest) -> Result<Vec<BlogPostSummary>> {
        let uid = uid_from_claims(req.claims.as_deref())?;
        let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.author_id == uid;
            include b.category;
            order_by b.published_at desc;
        ).to_list().await?;
        Ok(blogs.into_iter().map(BlogPostSummary::from).collect())
    }
}
```

## 2.4 创建操作（Create�?
```rust
#[inject]
#[async_trait]
impl IRequestHandler<CreateBlogPostRequest, BlogPostModel> for CreateBlogPostHandler {
    async fn handle(&self, req: CreateBlogPostRequest) -> Result<BlogPostModel> {
        // 1. �?claims 提取用户 ID
        let uid = req.claims.as_ref()
            .and_then(|c| c.subject().parse::<i32>().ok())
            .ok_or_else(|| Error::Http("Not authenticated".into()))?;

        // 2. 唯一性校�?        let set = ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let exists = set.filter(expr).first_or_default().await?;
        if exists.is_some() {
            return Err(Error::Http("Slug already exists".into()));
        }

        // 3. 构造实体并插入
        let now = chrono::Utc::now().timestamp();
        let mut blog = req.to_entity(uid, now);
        ctx.set::<Blog>().add(blog);
        ctx.save_changes().await?;
        // blog.id 已自动填�?—�?无需回查

        // 4. 仅当需要导航属性时，按主键回查
        let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or_else(|| Error::Internal("Blog vanished after insert".into()))?;

        tracing::info!("[Blog] Created: {} by {}", saved.slug, uid);
        Ok(saved.to_model())
    }
}
```

## 2.5 更新操作（Update�?
```rust
#[inject]
#[async_trait]
impl IRequestHandler<UpdateBlogPostRequest, BlogPostModel> for UpdateBlogPostHandler {
    async fn handle(&self, req: UpdateBlogPostRequest) -> Result<BlogPostModel> {
        // 1. 加载现有实体
        let set = ctx.set::<Blog>();
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

        // 4. 保存（detect_changes 仅标记实际变更的字段�?        ctx.set::<Blog>().detect_changes();
        ctx.save_changes().await?;

        // 5. 回查导航属性（按主键）
        let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
            include b.category;
            include b.author;
        ).first_or_default().await?
            .ok_or_else(|| Error::NotFound("Blog not found after update".into()))?;

        Ok(saved.to_model())
    }
}
```

## 2.6 删除操作（Delete �?软删除）

```rust
#[inject]
#[async_trait]
impl IRequestHandler<DeleteBlogPostRequest, String> for DeleteBlogPostHandler {
    async fn handle(&self, req: DeleteBlogPostRequest) -> Result<String> {
        // 1. 加载实体
        let set = ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.slug == req.slug);
        let mut blog = set.filter(expr).first_or_default().await?
            .ok_or_else(|| Error::NotFound(format!("Blog not found: {}", req.slug)))?;

        // 2. 权限校验
        let uid = uid_from_claims(req.claims.as_deref())?;
        let roles = roles_from_claims(req.claims.as_deref());
        if !is_admin(&roles) && blog.author_id != uid {
            return Err(Error::Http("Forbidden: not the author".into()));
        }

        // 3. 软删除（标记 + detect_changes + 保存�?        blog.is_deleted = true;
        blog.updated_at = chrono::Utc::now().timestamp();
        blog.updated_id = Some(uid);
        ctx.set::<Blog>().detect_changes();
        ctx.save_changes().await?;

        Ok(format!("Deleted blog {}", req.slug))
    }
}
```

> **前提**：启动时已注册全局查询过滤�?`has_query_filter(linq!(filter |b: Blog| !b.is_deleted))`�?> 标记 `is_deleted = true` 后该记录自动从所有查询中排除�?
## 2.7 错误处理

统一使用 `rust_webapp::Error` 类型，按场景映射�?
| 场景 | 错误类型 | 示例 |
|------|----------|------|
| 资源不存�?| `Error::NotFound` | `Error::NotFound(format!("Blog not found: {}", slug))` |
| 业务校验失败 | `Error::Http` | `Error::Http("Slug already exists".into())` |
| 权限不足 | `Error::Http` | `Error::Http("Forbidden: not the author".into())` |
| 数据库异�?| `Error::Internal` | `Error::Internal(format!("DB error: {}", e))` |
| 参数校验失败 | `Error::Validation` | `Error::Validation("Title is required".into())` |

**辅助函数（提�?claims 信息）：**

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

当业务逻辑复杂、需要跨 Handler 复用时，引入 Service 抽象�?
```rust
// contracts/blog.rs —�?契约�?pub trait IBlogService: Send + Sync {
    fn list_posts(&self) -> Result<Vec<BlogPostSummary>, String>;
    fn create_post(&self, req: CreateBlogPostRequest) -> Result<BlogPostModel, String>;
}

// handlers/blog_service.rs —�?实现�?#[derive(Inject)]
pub struct BlogService {
    ctx: Arc<dyn IDbContext>,
}

impl IBlogService for BlogService {
    fn list_posts(&self) -> Result<Vec<BlogPostSummary>, String> {
        todo!()
    }
}

// handlers/blog_handler.rs —�?�?Handler
#[derive(Inject)]
pub struct CreateBlogPostHandler {
    blog: Arc<dyn IBlogService>,  // 注入服务，而非直接注入 DbContext
}

#[inject]
#[async_trait]
impl IRequestHandler<CreateBlogPostRequest, BlogPostModel> for CreateBlogPostHandler {
    async fn handle(&self, req: CreateBlogPostRequest) -> Result<BlogPostModel> {
        self.blog.create_post(req)
            .map_err(|e| Error::Internal(e))
    }
}
```

**何时使用 Service 层：**

| 场景 | 建议 |
|------|------|
| 简�?CRUD，无�?Handler 复用 | 直接注入 `Arc<dyn IDbContext>` |
| 复杂业务逻辑，多 Handler 共享 | 抽取 `I...Service`，注�?`Arc<dyn I...Service>` |
| 需�?mock 测试 | 引入 Service 接口便于替换实现 |

## 2.9 变更追踪

`save_changes()` 之后需要知道的关键行为�?
| 行为 | 说明 |
|------|------|
| 自增 ID 回填 | `save_changes()` 后，实体�?`id` 字段已自动填充数据库生成的�?|
| 跟踪器清�?| 所有已追踪实体被清空，后续查询从数据库重新加载 |
| 导航属�?| 需要导航数据时，按**主键**（不�?slug/email）重新查询并 `include` |

```rust
// 新增后，id 已可�?ctx.set::<Blog>().add(blog);
ctx.save_changes().await?;
println!("�?ID: {}", blog.id); // �?已填�?
// 需要导航属性时，按主键回查
let enriched = linq!(ctx.set::<Blog>(), |b: Blog| b.id == blog.id;
    include b.category;
).first_or_default().await?;
```

## 2.10 导航属性加�?
```rust
// 贪婪加载（推荐）：一次查询加载所有关联数�?linq!(ctx.set::<Blog>(); include b.category; include b.author)
    .to_list().await?;

// 多级加载
linq!(ctx.set::<Blog>(); include b.posts then b.comments)
    .to_list().await?;

// 延迟加载（需�?options 中启�?use_lazy_loading(true)�?let posts = blog.posts.load().await?;
```

## 2.11 软删除（�?REF 层面�?
结合全局查询过滤�?+ 实体标记，三步完成：

**步骤 1：定义实�?*

```rust
#[derive(Debug, Clone, EntityType)]
#[table("articles")]
pub struct Article {
    #[primary_key] #[auto_increment]
    pub id: i32,
    pub title: String,
    pub is_deleted: bool,  // false = 活跃, true = 已删�?    pub updated_at: i64,
}
```

**步骤 2：启动时注册全局查询过滤�?*

```rust
// �?DbInitService::start() 中注册一�?ctx.model().entity::<Article>()
    .has_query_filter(linq!(filter |a: Article| !a.is_deleted));
// 对所有需要软删除的实体重复此操作
```

**步骤 3：执行软删除**

```rust
let query = ctx.set::<Article>().query();
let mut article = query.find(id).await?.unwrap();
article.is_deleted = true;
article.updated_at = now;
ctx.set::<Article>().detect_changes();  // 仅标记变更字�?ctx.save_changes().await?;
```

**管理员查看所有记录（含已删除）：**

```rust
ctx.set::<Article>().query_ignore_filters().to_list().await?;
```

> 完整软删除模板见 `templates/soft-delete.rs`，可运行示例�?`examples/soft_delete/src/main.rs`

## 2.12 查询 API 选择指南

| 场景 | 推荐 API | 示例 |
|------|----------|------|
| 过滤 + 排序 + 导航 | `linq!` Form B | `linq!(ctx.set::<T>(), \|t\| cond; include t.nav; order_by t.f).to_list()` |
| 仅主键查�?| `query().find(id)` | `let query = ctx.set::<T>().query(); query.find(42).await?` |
| 聚合（count/sum/avg�?| `linq!` Form B | `linq!(ctx.set::<T>(), \|t\| cond; count).await?` |
| 批量更新/删除 | `linq!` execute_update/delete | `linq!(ctx.set::<T>(), \|t\| cond; execute_delete).await?` |
| 忽略全局过滤�?| `query_ignore_filters()` | `ctx.set::<T>().query_ignore_filters().to_list()` |