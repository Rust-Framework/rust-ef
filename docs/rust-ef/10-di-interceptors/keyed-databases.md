<a id="多数据库-keyed-注册"></a>
# 多数据库 Keyed 注册与读写分离

> 本文同时是**读写分离教程**，覆盖：keyed 双 `DbContext` 注册、SQLite/MySQL/PostgreSQL 配置、完整订单 Handler、read-your-writes、副本延迟、只读账号、迁移与排错。

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

## 完整示例：订单读写 Handler

```rust
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("orders")]
pub struct Order {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[required]
    pub user_id: i32,
    pub total: f64,
}

#[derive(Inject)]
pub struct OrderHandler {
    #[inject(owned, key = "write")]
    write: DbContext,
    #[inject(owned, key = "read")]
    read: DbContext,
}

impl OrderHandler {
    /// 写：只在 write context 开事务。
    pub async fn create(&mut self, user_id: i32, total: f64) -> EFResult<()> {
        let txn = self.write.begin_transaction().await?;
        self.write.add::<Order>(Order {
            id: 0,
            user_id,
            total,
        });
        self.write.save_changes().await?;
        txn.commit().await?;
        Ok(())
    }

    /// 纯查询：走 read context，可接受副本延迟。
    pub async fn list(&mut self, user_id: i32) -> EFResult<Vec<Order>> {
        self.read
            .set::<Order>()
            .query()
            .filter(linq!(Order, |o| o.user_id == user_id))
            .to_list()
            .await
    }

    /// 强一致 / 写后立即读自己的写：走 write context。
    pub async fn get_consistent(&mut self, id: i32) -> EFResult<Option<Order>> {
        self.write.set::<Order>().query().find(id).await
    }
}
```

`create` 中的查询、`save_changes`、事务都在 write；`list` 不会占用主库连接；`get_consistent` 用于下单成功后的立即回读。

## 副本延迟与降级

| 策略 | 说明 |
|------|------|
| 关键读走主库 | 下单结果、支付状态、权限校验 |
| 列表/报表走副本 | 可接受秒级延迟 |
| 读副本失败降级主库 | `list` 失败时用 write 重试 |
| 不做“轮询副本直到看到新数据” | 延迟不可控；改用 write 或业务补偿 |

```rust
pub async fn list_with_fallback(&mut self, user_id: i32) -> EFResult<Vec<Order>> {
    match self.list(user_id).await {
        Ok(rows) => Ok(rows),
        Err(e) => {
            eprintln!("replica query failed, fallback to primary: {e}");
            self.write
                .set::<Order>()
                .query()
                .filter(linq!(Order, |o| o.user_id == user_id))
                .to_list()
                .await
        }
    }
}
```

监控：MySQL `SHOW REPLICA STATUS`（`Seconds_Behind_Source`），PostgreSQL `pg_stat_replication.replay_lag`。

## 只读账号

MySQL：

```sql
CREATE USER 'app_read'@'%' IDENTIFIED BY '<password>';
GRANT SELECT ON app.* TO 'app_read'@'%';
```

PostgreSQL：

```sql
CREATE ROLE app_read LOGIN PASSWORD '<password>';
GRANT CONNECT ON DATABASE app TO app_read;
GRANT USAGE ON SCHEMA public TO app_read;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO app_read;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO app_read;
```

- 迁移/DDL 只在主库执行，使用独立迁移账号；
- read context 使用只读账号，即使代码误写也会被数据库拒绝；
- SQLite 同文件没有账号隔离，只能靠代码约定和 code review。

## 迁移与测试

- 迁移在**主库**执行；MySQL/PostgreSQL 从库通过复制跟随 schema；
- 上线前确认复制已追平，避免 read context 查询旧列；
- SQLite 同一文件天然一致；
- 不要对 read context 调用 `ensure_created` / `ensure_deleted` / 迁移 API。

SQLite 同文件可以验证两个 context：

```rust
#[tokio::test]
async fn write_then_read_uses_separate_contexts() -> EFResult<()> {
    let services = test_services("target/test-rw.db")?;
    let mut write: DbContext = services.get_keyed_owned("write").unwrap();
    let mut read: DbContext = services.get_keyed_owned("read").unwrap();

    write.ensure_created().await?;
    let txn = write.begin_transaction().await?;
    write.add::<Order>(Order { id: 0, user_id: 1, total: 9.9 });
    write.save_changes().await?;
    txn.commit().await?;

    let rows = read.set::<Order>().query().to_list().await?;
    assert_eq!(rows.len(), 1);
    Ok(())
}
```

真实主从延迟/只读账号行为需要 MySQL/PostgreSQL 集成环境；CI 的 `mysql_crud_tests` / `postgres_crud_tests` 覆盖 provider 层事务与查询。

## 排错清单

| 现象 | 排查 |
|------|------|
| 读 context 报“没有实体” | 是否调用 `use_default_entity_model()` |
| 写后 read 看不到新数据 | 副本延迟；改用 write context |
| SQLite 写超时 | 写池是否 `max_pool_size(1)`；事务是否过长 |
| 读库权限错误 | 只读账号是否缺少 `SELECT` / schema 权限 |
| 两个 context 数据不一致 | 是否误以为共享事务/跟踪 |
| 迁移后 read 报列不存在 | 从库复制是否追平 |

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

> 本文档已包含完整读写分离 Handler、read-your-writes、副本延迟、只读账号与排错示例；无需跳转到其他章节。

下一节：[SaveChanges 拦截器](save-changes-interceptors.md)
