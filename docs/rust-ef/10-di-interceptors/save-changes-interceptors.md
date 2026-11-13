# SaveChanges 拦截器

拦截器允许在 `save_changes()` 的多个阶段注入横切逻辑，典型场景包含**审计日志**、**时间戳填充**、**验证**；软删除通常还需要应用代码配合（见下文）。

## 先选钩子：只读 vs 可变

`ISaveChangesInterceptor` 有四个钩子，全部带默认空实现：

| 钩子 | 上下文 | 能做什么 | 不能做什么 | 典型用途 |
|---|---|---|---|---|
| `on_saving_mut(&mut MutableSaveChangesContext)` | 可变更实体；在 change detection **之前**、事务开启之前执行 | 写服务端字段（`updated_at`/`updated_by`/规范化字段/派生列）；用 `for_each_entity_mut::<T>()`、`entries_mut()` + `entity_mut::<T>()` 拿 `&mut T` | 改条目状态（Added / Modified / Deleted） | 审计时间戳、服务端覆盖请求值 |
| `on_saving(&SaveChangesContext)` | 只读快照（`entries()` 只含 `type_id` / `type_name` / `state`） | 校验、计数、收集审计事件、返回 `Err` 中止 | 改实体字段 | 只读审计、前置校验 |
| `on_saved(...)` | 提交成功后的计数 | 记录成功日志 | 回滚或改数据 | 成功审计 |
| `on_save_failed(...)` | 原始错误 | 记录失败原因 | 抑制错误 | 失败审计 |

要点：

- `on_saving_mut` 的修改会被**同一次** `save_changes()` 的 change detection 检出，并在**同一个事务**里提交，不会另开事务；返回 `Err` 时在任何 SQL 之前中止（与 `on_saving` 相同的前置拒绝语义）。
- `on_saving_mut` 看到的 `state` 是 change detection **之前**的状态：`Unchanged` 表示"已跟踪的既有实体"，不代表"没有变更"。
- 只想写审计时间戳时，优先用内置 `AuditInterceptor`（见下文「时间戳管理」）。

## 实现拦截器（只读示例）

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

## 注册拦截器

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

## 软删除：拦截器 + 手动标记

软删除（Soft Delete）不物理删除行，而是将 `is_deleted` 标记为 `true`，配合全局查询过滤器
自动隐藏已删除记录。`on_saving_mut` 虽然能改实体字段，但**不能改变条目状态**：应用一旦调用
`remove()`，条目就是 `Deleted`，拦截器再写 `is_deleted` 也只会变成一条 DELETE。因此软删除
仍推荐"**手动标记 + 拦截器审计**"的分工：

| 角色 | 职责 | 实现方式 |
|------|------|----------|
| 应用代码 | 将 `is_deleted` 设为 `true` 并标记 Modified | `tracked_entries_mut()` + `detect_changes()` |
| 只读拦截器 | 记录审计日志（谁、何时、改了什么） | `on_saving` / `on_saved` |
| 可变拦截器（可选） | 统一填 `updated_at` / `updated_by` 等服务端字段 | `on_saving_mut`（见「时间戳管理」） |

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

// 2) 软删除：先把目标行载入 ChangeTracker，再改字段
ctx.load_all::<Article>().await?;           // 查询 + attach 为 Unchanged
for entry in ctx.set::<Article>().tracked_entries_mut() {
    if entry.title == "outdated" {
        entry.is_deleted = true;                  // 修改字段
    }
}
ctx.detect_changes();            // 快照比对 → 标记 Modified
ctx.save_changes().await?;                        // 生成 UPDATE，而非 DELETE
```

### 为什么不能用 `query().to_list()`？

`query().to_list()` 返回 `Vec<T>`，**不**把实体放入 ChangeTracker。`save_changes()` 之后
跟踪器会被清空，因此必须用 `load_all()`（查询 + `attach`）或手动 `attach()` 才能让
`tracked_entries_mut()` 看到条目。

### 管理员查询：绕过过滤器

```rust
// 普通查询：自动追加 AND is_deleted = false
let active = ctx.set::<Article>().query().to_list().await?;

// 管理员查询：查看全部记录（含已删除）
let all = ctx.set::<Article>().query_ignore_filters().to_list().await?;
```

### 审计拦截器

只读钩子（`on_saving`）不能改实体，适合记录"谁触发了保存"等审计信息；需要写服务端字段时用
`on_saving_mut`（见「时间戳管理」）。下面仍是只读审计示例：

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

完整可运行示例见 `examples/soft_delete`。

## 审计日志：缓冲区模式

拦截器无法直接访问数据库连接，因此不能在 `on_saving` 中写审计表。推荐采用
"**拦截器捕获 → 应用代码持久化**"的缓冲区模式：

1. 拦截器持有 `Arc<Mutex<Vec<AuditEvent>>>`，在 `on_saving` 中收集事件
2. `save_changes()` 返回后，应用代码 drain 缓冲区，写入 `audit_log` 表
3. 拦截器过滤掉 `AuditLog` 自身的事件，避免反馈循环

### 变更历史表设计

```sql
CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_type TEXT    NOT NULL,   -- 实体类型名（如 "Document"）
    action      TEXT    NOT NULL,   -- INSERT / UPDATE / DELETE
    occurred_at INTEGER NOT NULL,   -- Unix epoch 秒
    affected    INTEGER NOT NULL    -- 本次保存影响的行数
);
```

> 当前 `EntityEntryView` 只含 `type_name` + `state`，不支持字段级 diff。
> 如需记录变更前后的值，需扩展拦截器 API（未来迭代）。

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
            if entry.type_name == "AuditLog" { continue; }  // 防反馈循环
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

保存后持久化审计记录：

```rust
ctx.save_changes().await?;

// drain 缓冲区 → 写入 audit_log 表
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
ctx.save_changes().await?;   // 注意：拦截器会跳过 AuditLog 事件
```

完整可运行示例见 `examples/audit`。

## 时间戳管理：CreatedAt / UpdatedAt

2.3.0 起推荐用框架内置的审计填充器：实体实现 `IAuditable`，host 注册一次 `AuditInterceptor`，
`save_changes()` 时自动写服务端时间戳（INSERT 写 created + updated，UPDATE 只写 updated），
不需要在每个写路径手写。

### 推荐：`IAuditable` + `AuditInterceptor`

```rust
use rust_ef::audit::{AuditInterceptor, IAuditable};
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("documents")]
struct Document {
    #[primary_key] #[auto_increment] pub id: i32,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub updated_by: Option<String>,
}

// 只覆盖表里实际有的列；created_by 等不需要的 setter 用默认空实现。
impl IAuditable for Document {
    fn set_updated_at(&mut self, now_ms: i64) { self.updated_at = now_ms; }
    fn set_updated_by(&mut self, actor: Option<&str>) { self.updated_by = actor.map(str::to_string); }
    fn set_created_at(&mut self, now_ms: i64) { self.created_at = now_ms; }
}

// host 装配（一次）；clock / actor 是可替换端口
options.add_interceptor(AuditInterceptor::new().track::<Document>());
```

- 时间单位由 `IClock::now_ms` 定义为 **UNIX epoch 毫秒**；实体若存秒或 `DateTime`，在 setter 里转换。
- 未提供 actor 端口时默认 `NoAuditActor` 返回 `None`；需要"谁改了"就在 host 用 `.with_actor(...)` 注入请求身份端口。
- 只有需要自动填的实体才 `.track::<T>()`（Rust 没有 specialization，逐类型登记，集中在 host 装配处）。

### 手写可变钩子（不使用 `IAuditable` 时）

```rust
#[async_trait::async_trait]
impl ISaveChangesInterceptor for StampInterceptor {
    async fn on_saving_mut(&self, pending: &mut MutableSaveChangesContext<'_>) -> EFResult<()> {
        let now = now_epoch_ms();
        pending.for_each_entity_mut::<Document>(|state, doc| {
            if state == EntityState::Added { doc.created_at = now; }
            doc.updated_at = now;
        });
        Ok(())
    }
}
```

### 兼容：应用代码手动填充（2.2 及更早的写法）

仍可用 `load_all()` 载入跟踪器后手动戳记，再 `detect_changes()`：

```rust
ctx.load_all::<Document>().await?;
for doc in ctx.set::<Document>().tracked_entries_mut() {
    if doc.title == "old title" {
        doc.body = "new content".into();
    }
}
// 只对已修改的条目戳记 updated_at，避免不必要的 UPDATE
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

> **注意**：`stamp_updated_at` 只戳记匹配谓词的条目。如果对所有跟踪条目统一戳记，
> 会导致未修改的行也被标记为 Modified，产生多余的 UPDATE。

## 设计要点

| 实践 | 说明 |
|------|------|
| 拦截器按注册顺序执行 | 审计应在最前，验证在中间，软删除在最后 |
| 两个 pre-save 钩子都可中止提交 | `on_saving` / `on_saving_mut` 返回 `Err` 都会在任何 SQL 之前中止 |
| 拦截器不覆盖 `ExecuteUpdate/Delete` | 批量操作绕过 ChangeTracker，拦截器不触发 |
| 只读钩子不能改实体 | `SaveChangesContext` 只有 entry 视图；写服务端字段用 `on_saving_mut` |
| 可变钩子不能改条目状态 | 只能拿 `&mut` 实体；Added/Modified/Deleted 仍由 `add/update/remove` 决定 |
| 软删除 UPDATE 受查询过滤器约束 | WHERE 子句会 AND 过滤条件，防止跨租户/越权改写 |
| `save_changes()` 清空跟踪器 | 后续修改需重新 `load_all()` 或 `attach()` 才能被跟踪 |
| 审计拦截器需过滤自身实体 | 避免 `audit_log` 写入触发反馈循环 |
| 时间戳只戳记已修改条目 | 避免未变更行产生多余 UPDATE（用 `AuditInterceptor` 时由 change detection 兜底） |

下一章：[最佳实践与避坑](../11-best-practices/INDEX.md)
