# 全局查询过滤器

全局查询过滤器用于为所有查询自动附加 WHERE 条件，典型场景包括**软删除**和**多租户隔离**。

通过 `linq!` 宏的**形式 C**（`filter` 关键字）产出 `BoolExpr` 值，传入 `ModelBuilder.has_query_filter::<T>`。字符串 API（`has_query_filter("...")`）已移除。

## 注册过滤器

```rust
let mut ctx = DbContext::from_options(&options)?;

// 形式 C：linq!(filter |b: T| <bool_expr>) 产出 BoolExpr
ctx.model().has_query_filter::<Blog>(
    linq!(filter |b: Blog| b.deleted_at.is_null())
);

ctx.set::<Blog>();
ctx.ensure_created().await?;
```

也可以用 `BoolExpr::Filter(FilterCondition::with_values(...))` 直接构造，无需宏：

```rust
use rust_ef::query::{BoolExpr, FilterCondition};
use rust_ef::provider::DbValue;

ctx.model().has_query_filter::<Article>(
    BoolExpr::Filter(FilterCondition::with_values(
        "is_deleted", "=", vec![DbValue::Bool(false)],
    ))
);
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
ctx.model().has_query_filter::<Blog>(
    linq!(filter |b: Blog| b.deleted_at.is_null() && b.tenant_id == tenant_id)
);
```

形式 C 产出的 `BoolExpr` 是自包含的（参数值内联在 `FilterCondition::with_values` 中），无需依赖外部 `QueryBuilder` 状态。

## 绕过过滤器：管理员查询

```rust
// 普通查询：自动追加过滤器条件
let active = ctx.set::<Blog>().query().to_list().await?;

// 管理员查询：查看全部记录（含已删除 / 跨租户）
let all = ctx.set::<Blog>().query_ignore_filters().to_list().await?;
```

## 与 SaveChanges 的交互

查询过滤器不仅作用于 SELECT，也会注入到 `save_changes()` 生成的 **UPDATE / DELETE** 的 WHERE 子句中：

| 操作 | 是否过滤 | 说明 |
|------|----------|------|
| INSERT | 否 | 主键/外键由用户在 `add()` 前设置，过滤器不参与 |
| UPDATE | 是 | WHERE 子句 AND 过滤条件，越权改写返回 `ConcurrencyConflict` |
| DELETE | 是 | 同上，防止跨租户/跨软删除边界删除 |
| SELECT | 是 | 默认行为 |
| `query_ignore_filters()` | 否 | 管理员查询专用 |

> **安全提示**：过滤器是便利层，不是安全边界。权限敏感场景应在 Service / Handler 层显式校验。

## 与拦截器配合：软删除模式

软删除的推荐分工：

1. **全局过滤器**隐藏 `is_deleted = true` 的行（本文档）
2. **应用代码**手动把 `is_deleted` 设为 `true` 并调用 `detect_changes()` 标记 Modified
3. **审计拦截器**记录保存事件（拦截器只读，不能改实体字段）

```rust
// 1) 注册过滤器
ctx.model().has_query_filter::<Article>(
    BoolExpr::Filter(FilterCondition::with_values(
        "is_deleted", "=", vec![DbValue::Bool(false)],
    ))
);

// 2) 软删除：载入 → 改字段 → 标记 → 保存
ctx.set::<Article>().load_all().await?;
for entry in ctx.set::<Article>().tracked_entries_mut() {
    if entry.title == "outdated" {
        entry.is_deleted = true;
    }
}
ctx.set::<Article>().detect_changes();
ctx.save_changes().await?;   // 生成 UPDATE ... WHERE id=? AND is_deleted=false
```

UPDATE 的 WHERE 子句自动 AND 过滤条件，确保不会把已删除的行再次"取消删除"。完整示例见 `examples/soft_delete` 与 [SaveChanges 拦截器](../10-di-interceptors/save-changes-interceptors.md)。

## 设计要点

| 实践 | 说明 |
|------|------|
| 过滤器在 `set::<T>()` 前注册 | `DbSet` 创建时注入过滤器，之后修改 `ModelBuilder` 对已创建的 DbSet 无效 |
| 用 `linq!(filter ...)` 或 `BoolExpr::Filter` | 类型安全，参数化自动处理，无 SQL 注入风险 |
| 不要过度依赖过滤器做权限隔离 | 安全敏感逻辑应在 Handler/Service 层显式校验 |
| `save_changes()` 后跟踪器清空 | 后续修改需重新 `load_all()` 或 `attach()` |
| UPDATE/DELETE 同样受过滤器约束 | 越权写返回 `ConcurrencyConflict`，是安全特性 |

下一节：[原始 SQL 与已知限制](raw-sql-limitations.md)
