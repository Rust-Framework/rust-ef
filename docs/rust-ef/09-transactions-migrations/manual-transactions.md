# 手动事务、use_transaction 与 transaction

`save_changes()` 内部已包含事务，但某些场景需要**显式控制事务边界**，或者需要在同一个事务里组合多次写入、批量 DML 与查询。

## use_transaction

```rust
let result = ctx.use_transaction(|ctx| Box::pin(async move {
    // 以下所有语句共享同一个事务连接：
    let mut account = ctx.set::<Account>().query().find(from_id).await?.unwrap();
    account.balance -= amount;
    ctx.update::<Account>(account);
    ctx.save_changes().await?;

    let accounts = ctx.set::<Account>().query().to_list().await?;
    // 能读到上面未提交的更新

    Ok::<_, EFError>(())
})).await?;
```

## begin_transaction（手动句柄，独立事务）

`begin_transaction` 返回一个**独立**事务句柄，保持 v1.8 的语义：

```rust
let mut txn = ctx.begin_transaction().await?;

txn.connection()
    .execute("UPDATE accounts SET balance = balance - ? WHERE id = ?", &[
        DbValue::I32(amount),
        DbValue::I32(from_id),
    ])
    .await?;

txn.commit().await?;   // 或 txn.rollback().await?
```

- 句柄不会注册为 ambient：查询和 `save_changes()` **不会**加入它；`save_changes()` 仍自行提交。
- `transaction_mut()` 对它返回 `None`；直接使用句柄做 savepoint / isolation 操作。
- 未提交就 drop 会在后台回滚（best effort，需要 Tokio runtime）。
- 需要“查询、批量 DML、`save_changes` 全部加入同一个事务”时，用 [`use_transaction`](#use_transaction) 或 [`transaction`](#transaction回滚并返回业务结果)。
- 典型用途：一段 raw SQL、手动脚本，而不是 ORM 工作单元。

## 隔离级别

事务开始时指定隔离级别：

```rust
let txn = ctx.begin_transaction_with(IsolationLevel::Serializable).await?;

ctx.use_transaction_with(IsolationLevel::ReadCommitted, |ctx| {
    Box::pin(async move { /* ... */ Ok(()) })
}).await?;

ctx.transaction_with(IsolationLevel::RepeatableRead, |ctx| {
    Box::pin(async move { Ok(TxOutcome::Commit(())) })
}).await?;
```

- 隔离级别在事务开始时应用；这是 MySQL 唯一可移植的时机。
- SQLite 的 `*_with` 只支持 `Serializable`；其他级别返回 `EFError::Provider`，不会静默降级。
- PostgreSQL 的 `READ UNCOMMITTED` 会被数据库当作 `READ COMMITTED`。
- 句柄的 `set_isolation` 保留旧的 provider 行为（SQLite 切 PRAGMA、MySQL 发 `SET TRANSACTION`、PostgreSQL 生效）；它不保证跨库可移植，新代码优先用 `*_with`。

## 事务覆盖范围（当前版本起）

在 `use_transaction` / `transaction` 内，所有通过 `ctx` 发出的语句都复用同一条事务连接：

| 语句 | 是否在事务内 |
|------|:------------:|
| `save_changes()` | ✅（只 flush，不提交） |
| `to_list` / `count` / `first` / 导航加载 / `sql_query` | ✅ |
| `execute_update` / `execute_delete` | ✅ |
| 通过 `provider()` 取得的 raw SQL | ✅ |

因此“先批量改 A，再联查 A/B/C，再改 C”能读到本事务未提交的写；任一步失败，整体回滚。

> v1.8 及更早：查询、导航加载和批量 DML 会各自从连接池取新连接，读不到未提交写，`execute_update` / `execute_delete` 还会立即提交、不受事务保护。该缺陷已修复。

## transaction：回滚并返回业务结果

`use_transaction` 只有两种结果：`Ok` 提交、`Err` 回滚。业务上“条件不满足要撤销并返回结果”时，用 `transaction`：

```rust
let result = ctx.transaction(|ctx| Box::pin(async move {
    // 批量更新 A（在事务内）
    linq!(ctx.set::<Order>(), |o: Order| o.batch == batch_id;
        set o.checked, true;
        execute_update
    ).await?;

    // 联查 A/B/C —— 能看到未提交的批量更新
    let rows = linq!(ctx.set::<Order>(), |o: Order| o.batch == batch_id)
        .to_list()
        .await?;

    if !rows.iter().all(|o| o.is_valid()) {
        // 回滚，但返回成功结果
        return Ok(TxOutcome::Rollback(BizResult::Rejected));
    }

    ctx.save_changes().await?;   // flush，不提交
    Ok(TxOutcome::Commit(BizResult::Done))
})).await?;
```

| 闭包返回 | 数据库 | 上下文 | 外层结果 |
|----------|--------|--------|----------|
| `TxOutcome::Commit(v)` | commit | 保留 | `Ok(v)` |
| `TxOutcome::Rollback(v)` | rollback | `clear_tracking()`，detach 所有跟踪实体 | `Ok(v)` |
| `Err(e)` | rollback | `clear_tracking()`，detach 所有跟踪实体 | `Err(e)` |

注意：

- 回滚会 detach 所有跟踪实体（实体实例本身的值不变），避免后续 `save_changes` 把已回滚的写再次提交。
- `use_transaction` 的 `Err` 分支仍保留跟踪状态（兼容旧行为）；如果不打算重试，调用 `ctx.clear_tracking()`。
- 事务不可嵌套：`use_transaction` / `transaction` 内部再开事务会返回 `EFError::Transaction`。
- `ensure_created` / `ensure_deleted` / 迁移命令属于 DDL，仍直接走主库连接，不随 handler 事务回滚。
- 事务应尽量短：不要在网络请求、文件 IO 或长时间计算中持有事务；SQLite 单写连接下尤其明显。

## save_changes 的自动 savepoint

在**作用域事务**（`use_transaction` / `transaction`）内，每次 `save_changes()` 会：

1. 保存前 `SAVEPOINT rust_ef_save_N`；
2. 成功后 `RELEASE SAVEPOINT`；
3. 失败后 `ROLLBACK TO SAVEPOINT`，返回错误，外层事务仍可继续使用。

这样一次失败的 `save_changes` 不会把同一事务里之前已完成的写一起丢掉，也不会让 PostgreSQL 事务进入 aborted 状态。代价是每次保存多两条语句（与 EF Core 的行为一致）；不在作用域事务内的 `save_changes` 不创建 savepoint。

## 手动 savepoint

在 `use_transaction` 内可以通过 `transaction_mut()` 操作 savepoint；`begin_transaction` 返回的句柄可直接调用 `create_point`：

```rust
ctx.use_transaction(|ctx| Box::pin(async move {
    ctx.save_changes().await?;

    {
        let txn = ctx.transaction_mut().expect("ambient transaction");
        txn.create_point("before_more").await?;
    }

    // ... 可能失败的写入 ...
    {
        let txn = ctx.transaction_mut().expect("ambient transaction");
        txn.rollback_point("before_more").await?;
    }
    Ok(())
})).await?;
```

## 性能

- 显式事务会占用一个写连接直到提交/回滚；不要把网络请求、文件 IO 或长计算放进事务。
- 作用域事务内，每次**有变更**的 `save_changes()` 多两条语句（`SAVEPOINT` / `RELEASE`），与 EF Core 一致；空保存不会开启事务。
- 事务内的查询固定复用事务连接，不会额外占用连接池；事务外仍可并发使用多个连接。
- 单条 `save_changes()` 已自带事务；只有需要“多次写入 + 查询 + 条件回滚”时才需要再包一层显式事务。

## 设计要点

| 实践 | 说明 |
|------|------|
| 纯 ORM 增删改 | `save_changes()` 即可，自动事务 + 拦截器 + ChangeTracker |
| 多次写入/批量 DML/读自己的写 | `use_transaction` 或 `transaction` |
| 条件不满足要撤销并返回结果 | `transaction` + `TxOutcome::Rollback(v)` |
| 跨多个 DbContext | 分布式事务当前版本未内置 |
| 事务内不做 IO | 网络请求、文件操作会不可控地延长事务 |

下一节：[EnsureCreated 与 EnsureDeleted](ensure-created-deleted.md)
