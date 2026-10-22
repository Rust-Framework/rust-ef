# 手动事务：begin_transaction 与 use_transaction

`save_changes()` 内部已包含事务；需要显式控制事务边界时，使用 `begin_transaction()` 返回的 `ITransaction` 句柄（EF Core `IDbContextTransaction` 风格），或用 `use_transaction()` 闭包。

## begin_transaction：EF Core 风格

```rust
let txn = ctx.begin_transaction().await?;

ctx.add::<Blog>(blog);
ctx.save_changes().await?;          // 加入当前事务，不提交
let blogs = ctx.set::<Blog>().query().to_list().await?; // 也加入当前事务

txn.commit().await?;                // 或 txn.rollback().await?
```

- `begin_transaction` 返回 `Box<dyn ITransaction>`；
- 事务注册到 `DbContext`，查询、导航加载、批量 DML、`sql_query`、`save_changes()` 全部复用同一条事务连接；
- 提交/回滚由句柄显式控制；
- `commit` / `rollback` 消费句柄，类型上防止 use-after-commit；
- 未提交就 drop：连接空闲时同步 `discard`（MySQL/PostgreSQL 关闭连接，SQLite 执行 ROLLBACK）；连接正被语句使用时后台 rollback + discard；不会把带未提交事务的连接放回池。

## 业务回滚 + 返回结果

不再需要额外的 `TxOutcome`。先显式回滚，再返回业务结果：

```rust
let txn = ctx.begin_transaction().await?;

let from = ctx.set::<Account>().query().find(from_id).await?;
if from.as_ref().is_none_or(|a| a.balance < amount) {
    txn.rollback().await?;
    ctx.clear_tracking();
    return Ok(TransferOutcome::InsufficientFunds);
}

ctx.update::<Account>(/* ... */);
ctx.save_changes().await?;
txn.commit().await?;
Ok(TransferOutcome::Done)
```

- 业务拒绝：`txn.rollback().await?` + `clear_tracking()` + `Ok(...)`；
- 基础设施错误：`?` 提前返回；未消费的句柄 drop 时自动回滚；
- 需要重试时重新查询，不要复用回滚前的跟踪实体。

## use_transaction：闭包便利 API

如果不需要手动返回值，闭包更紧凑：

```rust
ctx.use_transaction(|ctx| {
    Box::pin(async move {
        // 查询、批量 DML、save_changes 都使用同一个事务连接
        let mut account = ctx.set::<Account>().query().find(id).await?.unwrap();
        account.balance -= amount;
        ctx.update::<Account>(account);
        ctx.save_changes().await?;
        Ok(())
    })
})
.await?;
```

- `Ok` 提交，`Err` 回滚并传播；
- 业务上需要“回滚但返回成功结果”时，用上面的 `begin_transaction` 手动句柄；
- 闭包签名需要 `Box::pin`，这是 Rust async 借用检查的折中。

## 隔离级别

```rust
let txn = ctx
    .begin_transaction_with(IsolationLevel::Serializable)
    .await?;
// ...
txn.commit().await?;

ctx.use_transaction_with(IsolationLevel::ReadCommitted, |ctx| {
    Box::pin(async move { /* ... */ Ok(()) })
})
.await?;
```

- MySQL 在 `START TRANSACTION` 之前设置；
- PostgreSQL 在 `BEGIN` 之后、首条查询之前设置；
- SQLite 只接受 `Serializable`；其他级别返回 `EFError::Provider`，不会静默降级；
- 事务开始后仍可通过句柄的 `set_isolation` 修改，但仅 PostgreSQL 在首条语句前有效；新代码优先用 `*_with`。

## 事务覆盖范围

在 `begin_transaction` / `use_transaction` 内，所有通过 `ctx` 发出的语句都复用同一条事务连接：

| 语句 | 是否在事务内 |
|------|:------------:|
| `save_changes()` | ✅（只 flush，不提交） |
| `to_list` / `count` / `first` / 导航加载 / `sql_query` | ✅ |
| `execute_update` / `execute_delete` | ✅ |
| 通过 `provider()` 取得的 raw SQL | ✅ |

因此“先批量改 A，再联查 A/B/C，再改 C”能读到本事务未提交的写；任一步失败，整体回滚。

> v1.8 及更早：查询、导航加载和批量 DML 会各自从连接池取新连接，读不到未提交写，`execute_update` / `execute_delete` 还会立即提交、不受事务保护。该缺陷已在 1.9 修复。
>
> 2.0 起 `begin_transaction` 也采用相同的作用域语义；1.x 的 standalone 行为已移除。

## 事务不可嵌套

```rust
let txn = ctx.begin_transaction().await?;
// ctx.begin_transaction().await?  -> EFError::Transaction
// ctx.use_transaction(...)         -> EFError::Transaction
txn.commit().await?;
```

同一 `DbContext` 同时只能有一个事务。需要嵌套语义时用 savepoint。

## save_changes 的自动 savepoint

在 `begin_transaction` / `use_transaction` 内，每次**有变更**的 `save_changes()` 会：

1. 保存前 `SAVEPOINT rust_ef_save_N`；
2. 成功后 `RELEASE SAVEPOINT`；
3. 失败后 `ROLLBACK TO SAVEPOINT`，返回错误，外层事务仍可继续使用。

这样一次失败的 `save_changes` 不会把同一事务里之前已完成的写一起丢掉，也不会让 PostgreSQL 事务进入 aborted 状态。代价是每次保存多两条语句（与 EF Core 的行为一致）；不在显式事务内的 `save_changes` 不创建 savepoint。

## 手动 savepoint

`begin_transaction` 返回的句柄直接支持 savepoint：

```rust
let mut txn = ctx.begin_transaction().await?;
ctx.save_changes().await?;

txn.create_point("before_more").await?;
// ... 可能失败的写入 ...
txn.rollback_point("before_more").await?;

txn.commit().await?;
```

> `use_transaction` 不暴露事务句柄；需要手动 savepoint / isolation 时使用 `begin_transaction` 返回的句柄。

## 异常与回滚：Rust 没有 try/finally

Rust 用 **`?` + RAII/Drop** 代替 `try/finally`：

```rust
let txn = ctx.begin_transaction().await?;

ctx.add::<Order>(order);
ctx.save_changes().await?;      // 任一步返回 Err
let rows = ctx.set::<Order>().query().to_list().await?;

txn.commit().await?;
```

- 任意 `?` 提前返回时，`txn` 离开作用域被 drop；
- drop 会先同步清除 context 的事务状态，再释放连接：空闲连接同步 `discard`，忙连接后台 rollback + discard；
- `discard` 由 provider 实现：MySQL/PostgreSQL 关闭连接，SQLite 同步 ROLLBACK；
- 因此不需要 `finally` 块，也不需要手写“每个分支都 rollback”。

### 需要确定性回滚时：显式 match

Drop 的释放可能走同步 `discard`，也可能在连接被语句占用时走后台任务；都不能在 Drop 中 `await`。如果必须在返回前确认回滚完成，用显式分支：

```rust
let txn = ctx.begin_transaction().await?;

let result: EFResult<_> = async {
    ctx.add::<Order>(order);
    ctx.save_changes().await?;
    Ok::<_, EFError>(order_id)
}
.await;

match result {
    Ok(value) => {
        txn.commit().await?;
        Ok(value)
    }
    Err(e) => {
        let _ = txn.rollback().await;
        Err(e)
    }
}
```

这就是 Rust 版的 `try/catch/finally`：

- `Ok`：显式 `commit`；
- `Err`：显式 `rollback`，再返回原错误；
- 如果 `rollback` 也失败，通常只记录日志，保留原始错误。

### `use_transaction` 已经封装了异常路径

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
- 不需要手写 `rollback`。

### panic、取消与运行时关闭

| 场景 | 行为 |
|------|------|
| 普通 `Err` / `?` | 句柄 drop；空闲连接同步 discard，忙连接后台 rollback + discard |
| panic（unwind） | drop 仍会执行；同上 |
| `panic = "abort"` | 进程终止，没有 drop；依赖数据库检测连接断开回滚 |
| future 被取消 | 句柄 drop；空闲连接同步 discard，忙连接后台处理 |
| Tokio runtime 已关闭 | 后台任务无法调度；`SharedTxn::drop` 兜底 discard 连接 |
| 事务开始后没有 Tokio runtime | 空闲连接同步 discard；忙连接由 `SharedTxn::drop` 兜底 |

生产代码不要只依赖 Drop：在关键路径显式 `commit()` / `rollback()`，Drop 作为最后一道保险。

### commit/rollback 本身失败

`commit()` / `rollback()` 返回 `Err` 时，事务状态可能已经不确定（例如网络在 COMMIT 响应前断开）：

- 向外传播错误，不要当作成功；
- 失败路径会尝试 rollback，并 `discard` 连接，避免把未知状态连接放回池；
- 当前 context 的事务状态会被清除，后续语句回到普通连接池；
- 业务层应把该操作视为失败，并根据幂等键/业务状态决定是否重试；
- 不要在同一个 context 上继续假设之前的事务仍然有效。

## 性能

- 显式事务会占用一个连接直到提交/回滚；不要把网络请求、文件 IO 或长计算放进事务。
- 每次**有变更**的 `save_changes()` 多两条 savepoint 语句，与 EF Core 一致；空保存不会开启事务。
- 事务内的查询固定复用事务连接，不会额外占用连接池；事务外仍可并发使用多个连接。
- SQLite 写池建议 `max_pool_size(1)`；事务内不要跨 context 查询副本。

## 设计要点

| 实践 | 说明 |
|------|------|
| 纯 ORM 增删改 | `save_changes()` 即可，自动事务 + 拦截器 + ChangeTracker |
| 多次写入/批量 DML/读自己的写 | `begin_transaction` 或 `use_transaction` |
| 条件不满足要撤销并返回结果 | `txn.rollback().await?` + `clear_tracking()` + `Ok(result)` |
| 跨多个 DbContext | 分布式事务当前版本未内置 |
| 事务内不做 IO | 网络请求、文件操作会不可控地延长事务 |

> 端到端业务示例（转账、批量 DML、测试与避坑）见 [事务实战：转账、批量 DML 与业务回滚](../12-tutorials/transactions.md)。
>
> 从 1.x 升级见 [2.0 事务 API 迁移](../13-migration/v2-transactions.md)。

下一节：[EnsureCreated 与 EnsureDeleted](ensure-created-deleted.md)
