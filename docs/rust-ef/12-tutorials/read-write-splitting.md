# 读写分离实战：主库写入、副本查询

本章把“两个 keyed `DbContext`”落到一个可运行的订单服务：

- 写 context 连主库，负责写入、事务、read-your-writes；
- 读 context 连从库/只读副本，负责纯查询；
- SQLite、MySQL、PostgreSQL 的配置差异；
- 副本延迟、只读账号、迁移、测试与排错。

> rust-ef **不做自动路由**。这是 EF Core 官方推荐的方式：由业务代码显式选择 read / write context。

## 1. 拓扑

```text
                    ┌────────────────────┐
  command / write ─►│ write DbContext     │──► primary (read-write)
                    │ pool, transaction   │
                    └────────────────────┘

                    ┌────────────────────┐
  query / read   ──►│ read DbContext      │──► replica (read-only)
                    │ separate pool       │
                    └────────────────────┘
```

关键规则：

| 场景 | 使用 |
|------|------|
| `save_changes`、批量 DML、事务 | `write` |
| 写后立即读自己的写 | `write` |
| 纯查询、列表、报表、可接受延迟 | `read` |
| 强一致查询 | `write` |

两个 context 不共享事务、ChangeTracker、连接池。不要期待“在 write 开事务，read 能读到未提交数据”。

## 2. SQLite 同文件配置

SQLite 没有网络主从，读写分离体现为“写池 1 + 读池独立”：

```rust
use rust_ef_sqlite::DbContextOptionsBuilderExt;
use std::time::Duration;

let services = ServiceCollection::new()
    .add_dbcontext_keyed("write", |options| {
        options.use_default_entity_model();
        options.use_sqlite_with("app.db", |sqlite| {
            sqlite
                .max_pool_size(1)
                .pool_acquire_timeout(Duration::from_secs(5));
        });
    })
    .add_dbcontext_keyed("read", |options| {
        options.use_default_entity_model();
        options.use_sqlite("app.db");
    })
    .build()?;
```

- 写池 `max_size = 1`：框架内串行化写，避免 SQLite 写锁竞争；
- 读池默认 8：并发查询；
- WAL + `busy_timeout` 由 provider 自动设置；
- 读取已提交数据；写事务进行中，读连接看到的是旧快照（SQLite WAL 语义）。

## 3. MySQL 主从配置

```rust
let services = ServiceCollection::new()
    .add_dbcontext_keyed("write", |options| {
        options.use_default_entity_model();
        options.use_mysql_with(primary_cs, |mysql| {
            mysql.max_pool_size(20);
            // 本地开发才用：mysql.insecure();
        });
    })
    .add_dbcontext_keyed("read", |options| {
        options.use_default_entity_model();
        options.use_mysql(replica_cs);
    })
    .build()?;
```

- `primary_cs` 指向主库，`replica_cs` 指向从库；
- 默认 TLS `Required`，本地明文需显式 `.insecure()`；
- 从库建议使用只读账号；
- 写池大小按写入并发设置，不要为了“和读池一样”盲目放大。

## 4. PostgreSQL 主从配置

```rust
let services = ServiceCollection::new()
    .add_dbcontext_keyed("write", |options| {
        options.use_default_entity_model();
        options.use_postgres_with(primary_cs, |pg| {
            pg.pool_size(20);
        });
    })
    .add_dbcontext_keyed("read", |options| {
        options.use_default_entity_model();
        options.use_postgres(replica_cs);
    })
    .build()?;
```

- 默认 TLS `Required`；本地开发可用 `.insecure()`；
- 只读副本通过流复制同步；
- 大查询放到 `read`，避免占用主库连接和 CPU。

## 5. Handler：显式选择 context

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
```

写入：只在 write context 开事务。

```rust
impl OrderHandler {
    pub async fn create(&mut self, user_id: i32, total: f64) -> EFResult<()> {
        self.write
            .transaction(|ctx| {
                Box::pin(async move {
                    ctx.add::<Order>(Order {
                        id: 0,
                        user_id,
                        total,
                    });
                    ctx.save_changes().await?;
                    Ok(TxOutcome::Commit(()))
                })
            })
            .await
    }
}
```

纯查询：走 read context。

```rust
impl OrderHandler {
    pub async fn list(&mut self, user_id: i32) -> EFResult<Vec<Order>> {
        self.read
            .set::<Order>()
            .query()
            .filter(linq!(Order, |o| o.user_id == user_id))
            .to_list()
            .await
    }
}
```

强一致读取 / 写后立即读：走 write context。

```rust
impl OrderHandler {
    pub async fn get_consistent(&mut self, id: i32) -> EFResult<Option<Order>> {
        self.write.set::<Order>().query().find(id).await
    }
}
```

> 如果 `create` 之后立刻调用 `list`（read context），从库可能还没同步到新订单。需要严格 read-your-writes 时调用 `get_consistent`，或让前端在下单成功后延迟刷新。

## 6. `use_default_entity_model` 为什么必须调用

读写副本是**同一套 schema、同一组实体**。但 keyed context 默认按 `#[context(key)]` 过滤实体；如果实体没有标 `#[context]`，keyed context 会认为“没有实体”，导致运行时错误。

`use_default_entity_model()` 让 keyed context 使用默认实体模型，忽略 key 过滤：

```rust
.add_dbcontext_keyed("write", |options| {
    options.use_default_entity_model(); // 必须
    options.use_mysql(primary_cs);
})
```

只有“不同数据库、不同实体集”的多租户/多库场景，才使用 `#[context("key")]` 做实体隔离；不要和 `use_default_entity_model()` 混用。

## 7. 副本延迟与降级

常见策略：

1. **关键读走主库**：下单结果、支付状态、权限校验；
2. **列表/报表走从库**：可接受秒级延迟；
3. **读从库失败时降级主库**：

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

4. **不要对 read context 做重试直到“看到新数据”**：副本延迟不可控；应使用 write context 或业务补偿。

监控项：

- MySQL：`SHOW REPLICA STATUS`（`Seconds_Behind_Source` / `Seconds_Behind_Master`）；
- PostgreSQL：`pg_stat_replication` 的 `replay_lag`；
- 读库连接失败率、慢查询、复制中断告警。

## 8. 只读账号

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

- 迁移/DDL 只在主库执行，使用单独的迁移账号；
- read context 使用只读账号，即使代码误写也会被数据库拒绝；
- SQLite 同文件没有账号隔离，只能靠代码约定和 code review。

## 9. 迁移与 schema 一致性

- 迁移在**主库**执行；
- MySQL/PostgreSQL 从库通过复制跟随 schema；
- 上线前确认复制已追平，避免 read context 查询到旧列；
- SQLite 同一文件天然一致；
- 不要对 read context 调用 `ensure_created` / `ensure_deleted` / 迁移 API。

## 10. 测试

SQLite 同文件可以直接验证两个 context：

```rust
#[tokio::test]
async fn write_then_read_uses_separate_contexts() -> EFResult<()> {
    // 使用临时文件，而不是 :memory:
    let services = test_services("target/test-rw.db")?;
    let mut write: DbContext = services.get_keyed_owned("write").unwrap();
    let mut read: DbContext = services.get_keyed_owned("read").unwrap();

    write.ensure_created().await?;
    write
        .transaction(|ctx| {
            Box::pin(async move {
                ctx.add::<Order>(Order { id: 0, user_id: 1, total: 9.9 });
                ctx.save_changes().await?;
                Ok(TxOutcome::Commit(()))
            })
        })
        .await?;

    // 同文件、已提交；读 context 能看到。
    let rows = read.set::<Order>().query().to_list().await?;
    assert_eq!(rows.len(), 1);
    Ok(())
}
```

真实主从延迟/只读账号行为需要 MySQL/PostgreSQL 集成环境；CI 的 `mysql_crud_tests` / `postgres_crud_tests` 覆盖 provider 层事务与查询。

## 11. 排错清单

| 现象 | 排查 |
|------|------|
| 读 context 报“没有实体” | 是否调用 `use_default_entity_model()` |
| 写后读 read 看不到新数据 | 副本延迟；改用 write context |
| SQLite 写超时 | 写池是否 `max_pool_size(1)`；事务是否过长 |
| 读库报权限错误 | 只读账号是否缺少 `SELECT` / schema 权限 |
| 两个 context 数据不一致 | 是否误以为共享事务/跟踪 |
| 迁移后 read 查询报列不存在 | 从库复制是否追平 |

## 12. 检查清单

- [ ] write / read 分别使用主库、从库连接串；
- [ ] 两个 context 都调用 `use_default_entity_model()`；
- [ ] 写、事务、read-your-writes 走 write；
- [ ] 纯查询、报表走 read；
- [ ] SQLite 写池 = 1，读池独立；
- [ ] 从库使用只读账号；
- [ ] 迁移只在主库执行；
- [ ] 监控复制延迟，关键读有主库降级策略；
- [ ] 不跨 context 期待共享事务或 ChangeTracker。

上一篇：[事务实战：转账、批量 DML 与业务回滚](transactions.md)
