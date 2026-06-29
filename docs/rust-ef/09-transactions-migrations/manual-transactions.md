# 手动事务与 use_transaction

`save_changes()` 内部已包含事务，但某些场景需要**显式控制事务边界**。

## use_transaction

```rust
let result = ctx.use_transaction(|conn| async move {
    // 在事务内执行自定义 SQL
    conn.execute("UPDATE accounts SET balance = balance - ? WHERE id = ?", &[
        DbValue::I32(amount),
        DbValue::I32(from_id),
    ]).await?;

    conn.execute("UPDATE accounts SET balance = balance + ? WHERE id = ?", &[
        DbValue::I32(amount),
        DbValue::I32(to_id),
    ]).await?;

    Ok::<_, EFError>(())
}).await?;
```

## 事务行为

- 成功：`conn.commit_transaction()` 自动提交
- 失败：`conn.rollback_transaction()` 自动回滚，原始错误向上传播

## SaveChanges 与手动事务的区别

| 场景 | 推荐方式 | 原因 |
|------|----------|------|
| 纯 ORM 增删改 | `save_changes()` | 自动事务、拦截器、ChangeTracker 全链路 |
| 混合 ORM + 原始 SQL | `use_transaction` | 需要统一控制多种操作的事务边界 |
| 跨多个 DbContext | 外部事务管理 | 分布式事务当前版本未内置 |

## 设计要点

| 实践 | 说明 |
|------|------|
| 尽量缩短事务 | 长事务占用连接池资源，增加死锁风险 |
| 事务内不做 IO | 网络请求、文件操作会不可控地延长事务 |

下一节：[EnsureCreated 与 EnsureDeleted](ensure-created-deleted.md)
