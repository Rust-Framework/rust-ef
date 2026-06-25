# 多数据库 Keyed 注册

当应用需要连接多个数据库时，使用 `add_dbcontext_keyed`。

## 读写分离示例

```rust
let provider = ServiceCollection::new()
    .add_dbcontext_keyed::<DbContext>("read", |options| {
        options.use_postgres("host=read-replica/db");
    })
    .add_dbcontext_keyed::<DbContext>("write", |options| {
        options.use_postgres("host=primary/db");
    })
    .build()
    .unwrap();

// 查询走读库
let read_ctx: Arc<dyn IDbContext> = provider.get_keyed("read");

// 写入走主库
let write_ctx: Arc<dyn IDbContext> = provider.get_keyed("write");
```

## 多租户示例

```rust
// 每个租户独立数据库
for tenant in &tenants {
    let key = format!("tenant_{}", tenant.id);
    svc.add_dbcontext_keyed::<DbContext>(&key, |options| {
        options.use_postgres(&tenant.connection_string);
    });
}
```

## 设计要点

| 实践 | 说明 |
|------|------|
| Keyed 上下文独立管理 | 每个 key 对应独立的 Provider 和连接池 |
| 读写分离在应用层控制 | `rust-ef` 不自动路由，需代码显式选择 |
| 注意连接池限制 | 每个 key 都会创建一组连接，避免 key 爆炸 |

下一节：[SaveChanges 拦截器](save-changes-interceptors.md)
