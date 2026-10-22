# 2.0 事务 API 迁移

2.0 把事务模型收敛到 **EF Core 风格的 `ITransaction`**，移除了 1.x 的 `transaction` 闭包和 `TxOutcome`。

## 1. 变更总览

| 1.x | 2.0 |
|-----|-----|
| `begin_transaction` 返回 standalone 句柄，查询/`save_changes` 不加入 | `begin_transaction` 返回 scoped `ITransaction`，查询/`save_changes` 自动加入 |
| `transaction(|ctx| -> TxOutcome<T>)` | 删除；用 `begin_transaction` + `commit`/`rollback` |
| `transaction_with(level, ...)` | 删除；用 `begin_transaction_with(level)` |
| `TxOutcome::Commit(v)` | `txn.commit().await?` 后返回 `v` |
| `TxOutcome::Rollback(v)` | `txn.rollback().await?` + `clear_tracking()` 后返回 `v` |
| `ITransaction::is_scoped` | 删除；所有 context 事务都是 scoped |
| `DbTransaction::new(conn)` | 删除；句柄只由 `DbContext` 创建 |
| `use_transaction` 的 `Ok`/`Err` | 保持不变：`Ok` 提交，`Err` 回滚 |

## 2. 从 `transaction` 迁移

1.x：

```rust
let result = ctx
    .transaction(|ctx| {
        Box::pin(async move {
            ctx.add::<Order>(order);
            ctx.save_changes().await?;

            if !valid {
                return Ok(TxOutcome::Rollback(Outcome::Rejected));
            }
            Ok(TxOutcome::Commit(Outcome::Done))
        })
    })
    .await?;
```

2.0：

```rust
let txn = ctx.begin_transaction().await?;

ctx.add::<Order>(order);
ctx.save_changes().await?; // 加入事务，不提交

if !valid {
    txn.rollback().await?;
    ctx.clear_tracking();
    return Ok(Outcome::Rejected);
}

txn.commit().await?;
Ok(Outcome::Done)
```

要点：

- `begin_transaction` 已注册到 context，不需要闭包；
- 查询、批量 DML、`save_changes` 都自动加入；
- 业务拒绝用显式 `rollback()`，不再需要第三态；
- `commit` / `rollback` 消费句柄，防止 use-after-commit；
- 提前 `?` 返回时，未消费的句柄 drop 自动回滚。

## 3. 从 `transaction_with` 迁移

1.x：

```rust
ctx.transaction_with(IsolationLevel::RepeatableRead, |ctx| {
    Box::pin(async move { /* ... */ Ok(TxOutcome::Commit(())) })
})
.await?;
```

2.0：

```rust
let txn = ctx
    .begin_transaction_with(IsolationLevel::RepeatableRead)
    .await?;
// ...
txn.commit().await?;
```

`use_transaction_with` 仍可用于闭包形式。

## 4. `use_transaction` 基本不变

```rust
ctx.use_transaction(|ctx| {
    Box::pin(async move {
        ctx.add::<Order>(order);
        ctx.save_changes().await?;
        Ok(())
    })
})
.await?;
```

- `Ok` 提交；
- `Err` 回滚并传播；
- 需要“回滚 + 返回业务值”时，改用 `begin_transaction` 手动句柄。

## 5. 旧代码依赖 standalone `begin_transaction` 怎么办

1.x 中：

```rust
let txn = ctx.begin_transaction().await?;
ctx.save_changes().await?; // 不加入，自行提交
txn.commit().await?;
```

2.0 中 `save_changes` 会加入该事务，不再自行提交。如果确实需要“独立于 context 的 raw SQL 事务”，直接用 provider 连接：

```rust
let mut conn = ctx.provider().get_connection().await?;
conn.begin_transaction().await?;
conn.execute("UPDATE ...", &[]).await?;
conn.commit_transaction().await?;
```

但这不参与 ORM 的 ChangeTracker / savepoint，不推荐用于业务写入；优先改成 2.0 的 scoped 事务。

## 6. 自定义 `ITransaction` 实现

2.0 删除了 `ITransaction::is_scoped`；所有 `DbContext` 返回的事务都按 scoped 处理。自定义实现只需保留：

- `commit` / `rollback`
- `create_point` / `release_point` / `rollback_point`
- `set_isolation`
- `connection`

`DbTransaction::new` 也不再公开；请通过 `DbContext::begin_transaction` 获取句柄。

`IAsyncConnection` 新增必实现的 `discard(self: Box<Self>)`：当事务异常 drop 或 commit/rollback 失败时，用它同步关闭连接，避免把带未提交事务的连接放回池。自定义 provider 必须实现：

- MySQL/PostgreSQL：从 pool 中 detach/take 连接，drop 即关闭；
- SQLite：同步执行 `ROLLBACK` 后再归还连接。

## 7. 异常与释放保证

2.0 的释放策略：

1. **显式 `commit` / `rollback`**：等待数据库响应；失败时 `discard` 连接，不回池。
2. **Drop 未完成事务**：
   - 连接空闲：同步 `discard`（MySQL/PostgreSQL 关闭连接，SQLite 执行 ROLLBACK）；
   - 连接正被语句使用：后台任务先 rollback 再 discard；
   - runtime 已关闭：`SharedTxn::drop` 作为最后兜底 discard。
3. **panic unwind**：同上；`panic = "abort"` 无法执行 Drop，只能依赖数据库检测连接断开回滚。

因此不再只依赖“后台 rollback 恰好执行”；连接不会带着未提交事务回到连接池。

## 8. 迁移检查清单

- [ ] 搜索 `TxOutcome`，改为显式 `rollback()` + 返回值；
- [ ] 搜索 `.transaction(` / `transaction_with`，改为 `begin_transaction` / `begin_transaction_with`；
- [ ] 检查依赖 `begin_transaction` standalone 语义的代码；
- [ ] 业务拒绝回滚后调用 `clear_tracking()`；
- [ ] 重试时重新查询，不复用回滚前的实体；
- [ ] 自定义 `ITransaction` 实现移除 `is_scoped`；
- [ ] 升级依赖到 `rust-ef = "2.0"`。

## 9. 相关文档

- [手动事务：begin_transaction 与 use_transaction](../09-transactions-migrations/manual-transactions.md)
- [事务实战：转账、批量 DML 与业务回滚](../12-tutorials/transactions.md)
- [读写分离实战：主库写入、副本查询](../12-tutorials/read-write-splitting.md)
