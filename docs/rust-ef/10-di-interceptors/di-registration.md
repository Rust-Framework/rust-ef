# rust-dicore 集成与注册模式

`rust-ef` 与 `rust-dicore` DI 容器深度集成，支持构造函数注入和接口解析。

## 基础注册

```rust
use rust_dicore::ServiceCollection;
use rust_ef::di::*;
use rust_ef::db_context::DbContext;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|options| {
        options.use_sqlite("data source=app.db");
    })
    .build()
    .unwrap();

// 解析为 trait object
let ctx: Arc<dyn IDbContext> = provider.get();
```

## 在 Handler 中注入

```rust
use rust_webapp::*;
use rust_ef::db_context::IDbContext;
use std::sync::Arc;

pub struct ListBlogsHandler {
    ctx: Arc<dyn IDbContext>,
}

#[handler(inject)]
#[async_trait]
impl IRequestHandler<ListBlogsRequest, Vec<BlogDto>> for ListBlogsHandler {
    async fn handle(&self, _req: ListBlogsRequest) -> Result<Vec<BlogDto>> {
        // 注意：IDbContext 是 object-safe，但 set() 需要 &mut DbContext
        // 实际使用时可向下转换或封装 Repository
        Ok(vec![])
    }
}
```

## Repository 封装模式

```rust
pub struct BlogRepository {
    ctx: DbContext,
}

impl BlogRepository {
    pub async fn list_high_rated(&mut self) -> EfResult<Vec<Blog>> {
        let set = self.ctx.set::<Blog>();
        let expr = linq!(|b: Blog| b.rating > 4);
        set.filter(expr).to_list().await
    }
}
```

## 设计要点

| 实践 | 说明 |
|------|------|
| `Arc<dyn IDbContext>` 适合跨层传递 | object-safe，可在 trait 边界中使用 |
| 实际查询时需要 `&mut DbContext` | 考虑在 Service/Repository 层持有具体类型 |
| 每个请求一个 DbContext | 避免长生命周期导致的并发问题 |

下一节：[多数据库 Keyed 注册](keyed-databases.md)
