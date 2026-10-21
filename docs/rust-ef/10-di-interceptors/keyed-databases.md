# 多数据库 Keyed 注册

当应用需要连接多个数据库时，使用 `add_dbcontext_keyed`。v1.1.0 起支持通过 `#[context("key")]` 属性实现**实体隔离**——每个 keyed `DbContext` 只管理属于自己的实体。

## 读写分离示例

rust-ef **不做自动路由**。按 EF Core 官方推荐的方式：用两个 keyed `DbContext` 分别承载读、写连接，由业务代码显式选择。

```rust
let provider = ServiceCollection::new()
    // 写上下文：主库
    .add_dbcontext_keyed("write", |options| {
        options.use_default_entity_model();
        options.use_mysql(primary_cs);
    })
    // 读上下文：从库 / 只读副本
    .add_dbcontext_keyed("read", |options| {
        options.use_default_entity_model();
        options.use_mysql(replica_cs);
    })
    .build()
    .unwrap();

#[derive(Inject)]
pub struct Handler {
    #[inject(owned, key = "write")]
    write: DbContext,
    #[inject(owned, key = "read")]
    read: DbContext,
}
```

SQLite 读写同一个文件时，用 `use_sqlite_with` 把写池限制为 1，读上下文保持默认池：

```rust
.add_dbcontext_keyed("write", |options| {
    options.use_default_entity_model();
    options.use_sqlite_with("app.db", |sqlite| {
        sqlite
            .max_pool_size(1)
            .pool_acquire_timeout(std::time::Duration::from_secs(5));
    });
})
.add_dbcontext_keyed("read", |options| {
    options.use_default_entity_model();
    options.use_sqlite("app.db");
})
```

> 读写副本是同一套 schema，必须调用 `use_default_entity_model()`，让两个 context 共享默认实体模型；否则 keyed context 会按 `#[context(key)]` 过滤实体，读/写上下文都发现不到业务实体。只有“不同数据库、不同实体集”的场景才使用 `#[context]` 过滤。

MySQL / PostgreSQL 需要控制写池大小时，用对应的 delegate 配置：

```rust
.add_dbcontext_keyed("write", |o| {
    o.use_default_entity_model();
    o.use_mysql_with(primary_cs, |mysql| {
        mysql.max_pool_size(20);
    });
})
.add_dbcontext_keyed("read", |o| {
    o.use_default_entity_model();
    o.use_mysql(replica_cs);
})

// PostgreSQL 同理：
// o.use_postgres_with(primary_cs, |pg| { pg.pool_size(20); });
```

选择规则：

| 场景 | 使用 |
|------|------|
| 写入、`save_changes` | `write` |
| 写后立即读自己的写（read-your-writes） | `write` |
| 显式事务 | `write` |
| 纯查询、可接受副本延迟 | `read` |
| 强一致查询 | `write` |

注意：

- 两个 context 不共享事务、变更跟踪或连接池；不要跨 context 期待同一个工作单元。
- 读副本存在同步延迟；写后立刻从 `read` 读取可能读到旧值。
- `read` context 不应执行写操作（EF Core 的读 context 也依赖约定，而非物理只读）。生产环境建议给 `read` 使用只读数据库账号（MySQL/PostgreSQL）；SQLite 同文件场景依赖代码约定。
- 没有 `use_*_read_write` 方法：读写拓扑由 keyed 注册和现有 `use_*` / `use_*_with_pool` 组合，不增加新的使用概念。

## 多租户示例

```rust
// 每个租户独立数据库
for tenant in &tenants {
    let key = format!("tenant_{}", tenant.id);
    svc.add_dbcontext_keyed(&key, |options| {
        options.use_postgres(&tenant.connection_string);
    });
}
```

## 实体隔离（v1.1.0+）

当不同 keyed 上下文管理不同的实体集合时，使用 `#[context("key")]` 属性标记实体归属：

```rust
use rust_ef::prelude::*;

// 默认上下文实体 —— 不标注 #[context]
#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key] #[auto_increment] pub id: i32,
    pub url: String,
}

// "logs" 上下文实体 —— 标注 #[context("logs")]
#[derive(Debug, Clone, EntityType)]
#[context("logs")]
#[table("log_entries")]
pub struct LogEntry {
    #[primary_key] #[auto_increment] pub id: i32,
    pub message: String,
}
```

配置也可按上下文隔离，使用 `#[entity(T, "key")]` 的第二参数：

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

注册后，每个 keyed DbContext 只自动发现属于自己的实体：

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

// Owned 解析（推荐）：
let mut primary: DbContext = provider.get_keyed_owned("primary").unwrap();
let mut logs: DbContext = provider.get_keyed_owned("logs").unwrap();
// primary 只管理 Blog（context_key = None）
// logs 只管理 LogEntry（context_key = Some("logs")）

// 共享解析（Arc<DbContext>，&self 访问）：
// let primary: Arc<DbContext> = scope.get_keyed("primary");
```

### 过滤规则

| 实体的 `context_key` | DbContext 的 key | 自动注册 |
|----------------------|-----------------|---------|
| `None`（无 `#[context]`） | `None`（默认上下文） | 是 |
| `None` | `Some("logs")` | 否 |
| `Some("logs")` | `None` | 否 |
| `Some("logs")` | `Some("logs")` | 是 |

> 不使用 `#[context]` 的实体归属于默认上下文（`context_key = None`），只会被默认 `DbContext` 发现。
> 读写副本场景调用 `use_default_entity_model()` 后，keyed context 的 `context_key` 视为 `None`，使用默认实体模型。
> 详见 [自动注册与发现](../02-quickstart/auto-registration.md#多数据库上下文隔离v110)。

## 设计要点

| 实践 | 说明 |
|------|------|
| Keyed 上下文独立管理 | 每个 key 对应独立的 Provider 和连接池 |
| 实体用 `#[context("key")]` 隔离 | 避免跨数据库实体污染，每个上下文只发现自己的实体 |
| 读写分离在应用层控制 | `rust-ef` 不自动路由，需代码显式选择 |
| SQLite 读写同文件 | 写 context 用 `use_sqlite_with("app.db", \|o\| { o.max_pool_size(1); })`，读 context 用 `use_sqlite("app.db")` |
| 注意连接池限制 | 每个 key 都会创建一组连接，避免 key 爆炸 |
| Owned 解析优先 | `get_keyed_owned()` / `#[inject(owned, key = "...")]` → `DbContext`，`&mut self` 访问 |

> 需要完整的读写分离业务示例（handler、read-your-writes、副本延迟、只读账号、排错）时，继续阅读 [读写分离实战：主库写入、副本查询](../12-tutorials/read-write-splitting.md)。

下一节：[SaveChanges 拦截器](save-changes-interceptors.md)
