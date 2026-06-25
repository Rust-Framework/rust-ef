# SaveChanges 拦截器

拦截器允许在 `save_changes()` 的多个阶段注入横切逻辑，典型场景包括**审计日志**、**软删除**和**验证**。

## 实现拦截器

```rust
use rust_ef::interceptor::{ISaveChangesInterceptor, SaveChangesContext, SaveChangesResultContext};
use rust_ef::error::EfResult;

struct AuditInterceptor;

#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EfResult<()> {
        tracing::info!(
            "Saving +{} ~{} -{}",
            ctx.added_count(),
            ctx.modified_count(),
            ctx.deleted_count()
        );
        Ok(())
    }

    async fn on_saved(&self, _ctx: &SaveChangesContext, result: &SaveChangesResultContext) -> EfResult<()> {
        tracing::info!("Saved: {} entities modified", result.total());
        Ok(())
    }

    async fn on_save_failed(&self, _ctx: &SaveChangesContext, err: &rust_ef::error::EfError) {
        tracing::error!("Save failed: {}", err);
    }
}
```

## 注册拦截器

```rust
let provider = ServiceCollection::new()
    .add_dbcontext::<DbContext>(|options| {
        options
            .use_sqlite("app.db")
            .add_interceptor(AuditInterceptor);
    })
    .build()
    .unwrap();
```

## 软删除拦截器示例

```rust
struct SoftDeleteInterceptor;

#[async_trait::async_trait]
impl ISaveChangesInterceptor for SoftDeleteInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EfResult<()> {
        // 将 Deleted 实体转换为 Modified，设置 deleted_at
        // 注意：当前版本需手动操作 ChangeTracker 条目
        Ok(())
    }
}
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 拦截器按注册顺序执行 | 审计应在最前，验证在中间，软删除在最后 |
| `on_saving` 可中止提交 | 返回 `Err` 会阻止事务开启 |
| 拦截器不覆盖 `ExecuteUpdate/Delete` | 批量操作绕过 ChangeTracker，拦截器不触发 |

下一章：[最佳实践与避坑](../11-best-practices/INDEX.md)
