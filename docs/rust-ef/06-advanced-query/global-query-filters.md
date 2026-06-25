# 全局查询过滤器

全局查询过滤器用于为所有查询自动附加 WHERE 条件，典型场景包括**软删除**和**多租户隔离**。

## 注册过滤器

```rust
let mut ctx = DbContext::from_options(&options)?;

// 为 Blog 注册全局过滤器
ctx.model().entity::<Blog>().has_query_filter("deleted_at IS NULL");

ctx.set::<Blog>();
ctx.ensure_created().await?;
```

## 效果

注册后，所有对 `Blog` 的查询都会自动追加 `AND deleted_at IS NULL`：

```rust
// 实际生成：SELECT * FROM blogs WHERE deleted_at IS NULL
let blogs = ctx.set::<Blog>().query().to_list().await?;

// 实际生成：SELECT * FROM blogs WHERE deleted_at IS NULL AND rating > ?
let filtered = ctx
    .set::<Blog>()
    .filter(linq!(|b: Blog| b.rating > 3))
    .to_list()
    .await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 过滤器在 `set::<T>()` 前注册 | `DbSet` 创建时注入过滤器，之后修改 `ModelBuilder` 对已创建的 DbSet 无效 |
| 使用原始 SQL 片段 | 确保片段语法与数据库方言兼容 |
| 不要过度依赖过滤器做权限隔离 | 安全敏感逻辑应在 Handler/Service 层显式校验 |

下一节：[原始 SQL 与已知限制](raw-sql-limitations.md)
