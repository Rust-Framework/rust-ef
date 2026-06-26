# 全局查询过滤器

全局查询过滤器用于为所有查询自动附加 WHERE 条件，典型场景包括**软删除**和**多租户隔离**。

通过 `linq!` 宏的**形式 C**（`filter` 关键字）产出 `BoolExpr` 值，传入 `ModelBuilder.has_query_filter`。字符串 API（`has_query_filter("...")`）已移除。

## 注册过滤器

```rust
let mut ctx = DbContext::from_options(&options)?;

// 形式 C：linq!(filter |b: T| <bool_expr>) 产出 BoolExpr
ctx.model().entity::<Blog>().has_query_filter(
    linq!(filter |b: Blog| b.deleted_at.is_null())
);

ctx.set::<Blog>();
ctx.ensure_created().await?;
```

## 效果

注册后，所有对 `Blog` 的查询都会自动追加 `AND deleted_at IS NULL`：

```rust
// 实际生成：SELECT * FROM blogs WHERE deleted_at IS NULL
let blogs = ctx.set::<Blog>().query().to_list().await?;

// 实际生成：SELECT * FROM blogs WHERE deleted_at IS NULL AND rating > ?
let filtered = linq!(ctx.set::<Blog>(), |b: Blog| b.rating > 3)
    .to_list()
    .await?;
```

## 多条件过滤器

```rust
// 软删除 + 多租户
ctx.model().entity::<Blog>().has_query_filter(
    linq!(filter |b: Blog| b.deleted_at.is_null() && b.tenant_id == tenant_id)
);
```

形式 C 产出的 `BoolExpr` 是自包含的（参数值内联在 `FilterCondition::with_values` 中），无需依赖外部 `QueryBuilder` 状态。

## 设计要点

| 实践 | 说明 |
|------|------|
| 过滤器在 `set::<T>()` 前注册 | `DbSet` 创建时注入过滤器，之后修改 `ModelBuilder` 对已创建的 DbSet 无效 |
| 用 `linq!(filter ...)` 而非原始 SQL | 类型安全，参数化自动处理，无 SQL 注入风险 |
| 不要过度依赖过滤器做权限隔离 | 安全敏感逻辑应在 Handler/Service 层显式校验 |

下一节：[原始 SQL 与已知限制](raw-sql-limitations.md)
