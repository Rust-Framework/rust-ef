# 多数据库 Keyed 注册

当应用需要连接多个数据库时，使用 `add_dbcontext_keyed`。v1.1.0 起支持通过 `#[context("key")]` 属性实�?*实体隔离**——每�?keyed `DbContext` 只管理属于自己的实体�?
## 读写分离示例

```rust
let provider = ServiceCollection::new()
    .add_dbcontext_keyed("read", |options| {
        options.use_postgres("host=read-replica/db");
    })
    .add_dbcontext_keyed("write", |options| {
        options.use_postgres("host=primary/db");
    })
    .build()
    .unwrap();

// 查询走读�?let read_ctx: Arc<DbContext> = provider.get_keyed("read");

// 写入走主�?let write_ctx: Arc<DbContext> = provider.get_keyed("write");
```

## 多租户示�?
```rust
// 每个租户独立数据�?for tenant in &tenants {
    let key = format!("tenant_{}", tenant.id);
    svc.add_dbcontext_keyed(&key, |options| {
        options.use_postgres(&tenant.connection_string);
    });
}
```

## 实体隔离（v1.1.0�?
当不�?keyed 上下文管理不同的实体集合时，使用 `#[context("key")]` 属性标记实体归属：

```rust
use rust_ef::prelude::*;

// 默认上下文实�?—�?不标�?#[context]
#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key] #[auto_increment] pub id: i32,
    pub url: String,
}

// "logs" 上下文实�?—�?标注 #[context("logs")]
#[derive(Debug, Clone, EntityType)]
#[context("logs")]
#[table("log_entries")]
pub struct LogEntry {
    #[primary_key] #[auto_increment] pub id: i32,
    pub message: String,
}
```

配置也可按上下文隔离，使�?`#[entity(T, "key")]` 的第二参数：

```rust
#[derive(Default)]
pub struct LogEntryConfig;

#[entity(LogEntry, "logs")]
impl IEntityTypeConfiguration<LogEntry> for LogEntryConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, LogEntry>) {
        entity.property_named("message").has_index();
    }
}
```

注册后，每个 keyed DbContext 只自动发现属于自己的实体�?
```rust
let provider = ServiceCollection::new()
    .add_dbcontext_keyed("primary", |options| {
        options.use_postgres("host=primary/db");
    })
    .add_dbcontext_keyed("logs", |options| {
        options.use_sqlite("logs.db");
    })
    .build()
    .unwrap();

let primary: Arc<DbContext> = provider.get_keyed("primary");
let logs: Arc<DbContext> = provider.get_keyed("logs");
// primary 只管�?Blog（context_key = None�?// logs 只管�?LogEntry（context_key = Some("logs")�?```

### 过滤规则

| 实体�?`context_key` | DbContext �?key | 自动注册 |
|----------------------|-----------------|---------|
| `None`（无 `#[context]`�?| `None`（默认上下文�?| �?|
| `None` | `Some("logs")` | �?|
| `Some("logs")` | `None` | �?|
| `Some("logs")` | `Some("logs")` | �?|

> 不使�?`#[context]` 的实体归属于默认上下文（`context_key = None`），只会被默�?`DbContext` 发现�?
详见 [自动注册与发现](../02-quickstart/auto-registration.md#多数据库上下文隔离v110)�?
## 设计要点

| 实践 | 说明 |
|------|------|
| Keyed 上下文独立管�?| 每个 key 对应独立�?Provider 和连接池 |
| 实体�?`#[context("key")]` 隔离 | 避免跨数据库实体污染，每个上下文只发现自己的实体 |
| 读写分离在应用层控制 | `rust-ef` 不自动路由，需代码显式选择 |
| 注意连接池限�?| 每个 key 都会创建一组连接，避免 key 爆炸 |

下一节：[SaveChanges 拦截器](save-changes-interceptors.md)
