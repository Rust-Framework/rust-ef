# rust-dicore 集成与注册模式

`rust-ef` 与 `rust-dicore` DI 容器深度集成，支持构造函数注入和 owned/shared 两种解析模式。

## 基础注册

```rust
use rust_dicore::ServiceCollection;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

// 推荐：owned 解析，直接 &mut self 访问
let mut ctx: DbContext = provider.get_owned();

// 或：共享解析（Arc<DbContext>，&self 访问，同一 scope 内共享）
// let scope = provider.create_scope();
// let ctx: Arc<DbContext> = scope.get();
```

## 在 Handler 中注入

`#[derive(Inject)]` 自动检测 bare `T` 字段并使用 `get_owned()` 解析，Handler 方法使用 `&mut self`：

```rust
use rust_webapp::*;
use rust_ef::db_context::DbContext;

#[derive(Inject)]
pub struct ListBlogsHandler {
    ctx: DbContext,  // bare T → owned 解析
}

#[inject(scoped)]
#[async_trait]
impl IRequestHandler<ListBlogsRequest, Vec<BlogDto>> for ListBlogsHandler {
    async fn handle(&mut self, _req: ListBlogsRequest) -> Result<Vec<BlogDto>> {
        let blogs = self.ctx.set::<Blog>().query().to_list().await?;
        Ok(blogs.into_iter().map(BlogDto::from).collect())
    }
}
```

## Repository 封装模式

```rust
pub struct BlogRepository {
    ctx: DbContext,
}

impl BlogRepository {
    pub async fn list_high_rated(&mut self) -> EFResult<Vec<Blog>> {
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.rating > 4);
        set.filter(expr).to_list().await
    }
}
```

## 设计要点

| 实践 | 说明 |
|------|------|
| Owned 解析（推荐） | `get_owned()` → `DbContext`，`&mut self` 访问，无需锁 |
| Shared 解析 | `scope.get()` → `Arc<DbContext>`，`&self` 访问，同一 scope 内共享 |
| 每个请求一个 DbContext | Scoped 生命周期，避免跨请求跟踪污染 |
| `#[derive(Inject)]` 自动检测 | bare `T` 字段 → owned；`Arc<T>` 字段 → shared |

下一节：[多数据库 Keyed 注册](keyed-databases.md)
