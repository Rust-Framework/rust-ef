# rust-dicore 集成与注册模�?

`rust-ef` �?`rust-dicore` DI 容器深度集成，支持构造函数注入和接口解析�?

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

// 解析�?trait object
let ctx: Arc<DbContext> = provider.get();
```

## �?Handler 中注�?

```rust
use rust_webapp::*;
use rust_ef::db_context::DbContext;
use std::sync::Arc;

pub struct ListBlogsHandler {
    ctx: Arc<DbContext>,
}

#[handler(inject)]
#[async_trait]
impl IRequestHandler<ListBlogsRequest, Vec<BlogDto>> for ListBlogsHandler {
    async fn handle(&self, _req: ListBlogsRequest) -> Result<Vec<BlogDto>> {
        // 注意：DbContext �?具体上下文类型，但 set() 需�?&mut DbContext
        // 实际使用时可向下转换或封�?Repository
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
| `Arc<DbContext>` 适合跨层传�?| 具体上下文类型，可�?trait 边界中使�?|
| 实际查询时需�?`&mut DbContext` | 考虑�?Service/Repository 层持有具体类�?|
| 每个请求一�?DbContext | 避免长生命周期导致的并发问题 |

下一节：[多数据库 Keyed 注册](keyed-databases.md)
