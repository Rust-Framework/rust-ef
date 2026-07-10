# SaveChanges 拦截�?

拦截器允许在 `save_changes()` 的多个阶段注入横切逻辑，典型场景包�?*审计日志**�?*软删�?*�?*验证**�?

## 实现拦截�?

```rust
use rust_ef::interceptor::{ISaveChangesInterceptor, SaveChangesContext, SaveChangesResultContext};
use rust_ef::error::EFResult;

struct AuditInterceptor;

#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        tracing::info!(
            "Saving +{} ~{} -{}",
            ctx.added_count(),
            ctx.modified_count(),
            ctx.deleted_count()
        );
        Ok(())
    }

    async fn on_saved(&self, _ctx: &SaveChangesContext, result: &SaveChangesResultContext) -> EFResult<()> {
        tracing::info!("Saved: {} entities modified", result.total());
        Ok(())
    }

    async fn on_save_failed(&self, _ctx: &SaveChangesContext, err: &rust_ef::error::EFError) {
        tracing::error!("Save failed: {}", err);
    }
}
```

## 注册拦截�?

```rust
let provider = ServiceCollection::new()
    .add_dbcontext(|options| {
        options
            .use_sqlite("app.db")
            .add_interceptor(AuditInterceptor);
    })
    .build()
    .unwrap();
```

## 软删除：拦截�?+ 手动标记

软删除（Soft Delete）不物理删除行，而是�?`is_deleted` 标记�?`true`，配合全局查询过滤�?
自动隐藏已删除记录。由于当�?`SaveChangesContext` 只暴�?*只读**�?`EntityEntryView`（含
`type_id` / `type_name` / `state`，不含实体引用），拦截器**无法**直接修改实体字段。因�?
推荐采用"**手动标记 + 拦截器审�?*"的分工模式：

| 角色 | 职责 | 实现方式 |
|------|------|----------|
| 应用代码 | �?`is_deleted` 设为 `true` 并标�?Modified | `tracked_entries_mut()` + `detect_changes()` |
| 拦截�?| 记录审计日志（谁、何时、改了什么） | `on_saving` / `on_saved` |

### 完整模式

```rust
use rust_ef::prelude::*;
use rust_ef::query::{BoolExpr, FilterCondition};
use rust_ef::provider::DbValue;

#[derive(Debug, Clone, EntityType)]
#[table("articles")]
struct Article {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub title: String,
    /// true = 已软删除
    pub is_deleted: bool,
}

// 1) 注册全局查询过滤器：只看到未删除记录
let soft_delete_filter = BoolExpr::Filter(FilterCondition::with_values(
    "is_deleted", "=", vec![DbValue::Bool(false)],
));
ctx.model().has_query_filter::<Article>(soft_delete_filter);
ctx.set::<Article>();
ctx.ensure_created().await?;

// 2) 软删除：先把目标行载�?ChangeTracker，再改字�?
ctx.load_all::<Article>().await?;           // 查询 + attach �?Unchanged
for entry in ctx.set::<Article>().tracked_entries_mut() {
    if entry.title == "outdated" {
        entry.is_deleted = true;                  // 修改字段
    }
}
ctx.detect_changes();            // 快照比对 �?标记 Modified
ctx.save_changes().await?;                        // 生成 UPDATE，而非 DELETE
```

### 为什么不能用 `query().to_list()`�?

`query().to_list()` 返回 `Vec<T>` �?*�?*把实体放�?ChangeTracker。`save_changes()` 之后
跟踪器会被清空，因此必须�?`load_all()`（查�?+ `attach`）或手动 `attach()` 才能�?
`tracked_entries_mut()` 看到条目�?

### 管理员查询：绕过过滤�?

```rust
// 普通查询：自动追加 AND is_deleted = false
let active = ctx.set::<Article>().query().to_list().await?;

// 管理员查询：查看全部记录（含已删除）
let all = ctx.set::<Article>().query_ignore_filters().to_list().await?;
```

### 审计拦截�?

拦截器不适合做软删除本身（无法改实体），但非常适合记录"谁触发了保存"等审计信息：

```rust
struct AuditInterceptor;

#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        tracing::info!(
            "Saving +{} ~{} -{}",
            ctx.added_count(), ctx.modified_count(), ctx.deleted_count()
        );
        Ok(())
    }
    async fn on_saved(
        &self, _ctx: &SaveChangesContext, result: &SaveChangesResultContext,
    ) -> EFResult<()> {
        tracing::info!("Saved: {} entities", result.total());
        Ok(())
    }
    async fn on_save_failed(&self, _ctx: &SaveChangesContext, err: &rust_ef::error::EFError) {
        tracing::error!("Save failed: {}", err);
    }
}
```

完整可运行示例见 `examples/soft_delete`�?

## 审计日志：缓冲区模式

拦截器无法直接访问数据库连接，因此不能在 `on_saving` 中写审计表。推荐采�?
"**拦截器捕�?�?应用代码持久�?*"的缓冲区模式�?

1. 拦截器持�?`Arc<Mutex<Vec<AuditEvent>>>`，在 `on_saving` 中收集事�?
2. `save_changes()` 返回后，应用代码 drain 缓冲区，写入 `audit_log` �?
3. 拦截器过滤掉 `AuditLog` 自身的事件，避免反馈循环

### 变更历史表设�?

```sql
CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type TEXT    NOT NULL,   -- 实体类型名（�?"Document"�?
    action      TEXT    NOT NULL,   -- INSERT / UPDATE / DELETE
    occurred_at INTEGER NOT NULL,   -- Unix epoch �?
    affected    INTEGER NOT NULL    -- 本次保存影响的行�?
);
```

> 当前 `EntityEntryView` 只含 `type_name` + `state`，不支持字段�?diff�?
> 如需记录变更前后的值，需扩展拦截�?API（未来迭代）�?

### 实现

```rust
use std::sync::{Arc, Mutex};
use rust_ef::entity::EntityState;

#[derive(Clone)]
struct AuditEvent {
    entity_type: String,
    action: &'static str,
    occurred_at: i64,
}

struct AuditInterceptor {
    events: Arc<Mutex<Vec<AuditEvent>>>,
}

#[async_trait::async_trait]
impl ISaveChangesInterceptor for AuditInterceptor {
    async fn on_saving(&self, ctx: &SaveChangesContext) -> EFResult<()> {
        let now = now_epoch();
        let mut buf = self.events.lock().unwrap();
        for entry in ctx.entries() {
            if entry.type_name == "AuditLog" { continue; }  // 防反馈循�?
            let action = match entry.state {
                EntityState::Added    => "INSERT",
                EntityState::Modified => "UPDATE",
                EntityState::Deleted  => "DELETE",
                _ => continue,
            };
            buf.push(AuditEvent {
                entity_type: entry.type_name.clone(),
                action, occurred_at: now,
            });
        }
        Ok(())
    }
    // on_saved / on_save_failed 省略
}
```

保存后持久化审计记录�?

```rust
ctx.save_changes().await?;

// drain 缓冲�?�?写入 audit_log �?
let drained: Vec<AuditEvent> = audit_buffer.lock().unwrap().drain(..).collect();
for ev in &drained {
    ctx.add::<AuditLog>(AuditLog {
        id: 0,
        entity_type: ev.entity_type.clone(),
        action: ev.action.to_string(),
        occurred_at: ev.occurred_at,
        affected: 1,
    });
}
ctx.save_changes().await?;   // 注意：拦截器会跳�?AuditLog 事件
```

完整可运行示例见 `examples/audit`�?

## 时间戳管理：CreatedAt / UpdatedAt

拦截器无法修改实体字段，因此 `created_at` / `updated_at` 需在应用代码中手动填充�?

### 插入�?

构造实体时直接设置两个时间戳：

```rust
let now = now_epoch();
ctx.add::<Document>(Document {
    id: 0,
    title: "Design Doc".into(),
    body: "...".into(),
    created_at: now,   // 插入时设�?
    updated_at: now,   // 插入时设�?
});
ctx.save_changes().await?;
```

### 更新�?

�?`load_all()` 载入跟踪器，修改字段后用谓词匹配戳记 `updated_at`�?

```rust
ctx.load_all::<Document>().await?;
for doc in ctx.set::<Document>().tracked_entries_mut() {
    if doc.title == "old title" {
        doc.body = "new content".into();
    }
}
// 只对已修改的条目戳记 updated_at，避免不必要�?UPDATE
stamp_updated_at(&mut ctx, |d| d.title == "old title");
ctx.save_changes().await?;

fn stamp_updated_at<F>(ctx: &mut DbContext, predicate: F)
where F: Fn(&Document) -> bool {
    let now = now_epoch();
    for doc in ctx.set::<Document>().tracked_entries_mut() {
        if predicate(doc) { doc.updated_at = now; }
    }
    ctx.detect_changes();
}
```

> **注意**：`stamp_updated_at` 只戳记匹配谓词的条目。如果对所有跟踪条目统一戳记�?
> 会导致未修改的行也被标记�?Modified，产生多余的 UPDATE�?

## 设计要点

| 实践 | 说明 |
|------|------|
| 拦截器按注册顺序执行 | 审计应在最前，验证在中间，软删除在最�?|
| `on_saving` 可中止提�?| 返回 `Err` 会阻止事务开�?|
| 拦截器不覆盖 `ExecuteUpdate/Delete` | 批量操作绕过 ChangeTracker，拦截器不触�?|
| 拦截器无法修改实体字�?| `SaveChangesContext` 只读；软删除/时间戳须在应用代码中手动标记 |
| 软删�?UPDATE 受查询过滤器约束 | WHERE 子句�?AND 过滤条件，防止跨租户/越权改写 |
| `save_changes()` 清空跟踪�?| 后续修改需重新 `load_all()` �?`attach()` 才能被跟�?|
| 审计拦截器需过滤自身实体 | 避免 `audit_log` 写入触发反馈循环 |
| 时间戳只戳记已修改条�?| 避免未变更行产生多余 UPDATE |

下一章：[最佳实践与避坑](../11-best-practices/INDEX.md)
