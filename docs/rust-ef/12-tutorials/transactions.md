# 事务实战：转账、批量 DML 与业务回滚

本章用一个“账户转账”场景，把 EF Core 风格的 `begin_transaction` / `ITransaction` 串成完整业务代码，并覆盖：

- `begin_transaction` 返回 `ITransaction`，显式 `commit` / `rollback`；
- 批量 DML、查询、`save_changes` 在同一个事务里；
- 业务拒绝时回滚并返回业务结果；
- 隔离级别、savepoint；
- 测试与常见坑。

## 1. 场景

```text
accounts
  id       INTEGER PRIMARY KEY AUTO_INCREMENT
  owner    TEXT NOT NULL
  balance  DOUBLE NOT NULL
```

业务规则：

1. 转出账户余额必须足够；
2. 转出、转入、写流水必须在同一个事务；
3. 余额不足时回滚，但向外返回“余额不足”这一业务结果，而不是抛错；
4. 任何数据库错误都回滚并向上传播。

## 2. 实体与 Handler

```rust
use rust_ef::prelude::*;

#[derive(Debug, Clone, EntityType)]
#[table("accounts")]
pub struct Account {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[required]
    pub owner: String,
    pub balance: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferOutcome {
    Done,
    InsufficientFunds,
}

#[derive(Inject)]
pub struct TransferHandler {
    #[inject(owned)]
    ctx: DbContext,
}
```

## 3. begin_transaction：显式提交/回滚

```rust
impl TransferHandler {
    pub async fn transfer(
        &mut self,
        from_id: i32,
        to_id: i32,
        amount: f64,
    ) -> EFResult<TransferOutcome> {
        let txn = self.ctx.begin_transaction().await?;

        let mut from = self
            .ctx
            .set::<Account>()
            .query()
            .find(from_id)
            .await?
            .ok_or_else(|| EFError::query(format!("account {from_id} not found")))?;

        let mut to = self
            .ctx
            .set::<Account>()
            .query()
            .find(to_id)
            .await?
            .ok_or_else(|| EFError::query(format!("account {to_id} not found")))?;

        if from.balance < amount {
            // 显式回滚，然后返回成功的业务结果。
            txn.rollback().await?;
            self.ctx.clear_tracking();
            return Ok(TransferOutcome::InsufficientFunds);
        }

        from.balance -= amount;
        to.balance += amount;
        self.ctx.update::<Account>(from);
        self.ctx.update::<Account>(to);

        // 在事务内 flush：不提交，由 txn 统一提交/回滚。
        self.ctx.save_changes().await?;

        txn.commit().await?;
        Ok(TransferOutcome::Done)
    }
}
```

关键点：

- `begin_transaction` 返回 `Box<dyn ITransaction>`；
- 事务注册在 `DbContext` 上，查询、`save_changes`、批量 DML 都自动加入；
- 业务拒绝用 `txn.rollback().await?`，不再需要 `TxOutcome`；
- `rollback` / `commit` 消费句柄，类型上防止 use-after-commit；
- 提前 `?` 返回时，未消费的句柄 drop 会自动回滚。

## 4. 批量 DML + 查询 + save_changes

```rust
#[derive(Debug, Clone, EntityType)]
#[table("orders")]
pub struct Order {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    #[required]
    pub batch: i32,
    pub checked: bool,
    pub amount: f64,
}

impl Order {
    fn is_valid(&self) -> bool {
        self.amount > 0.0
    }
}

impl TransferHandler {
    pub async fn check_batch(&mut self, batch: i32) -> EFResult<bool> {
        let txn = self.ctx.begin_transaction().await?;

        // 1) 批量 DML 加入事务，不立即提交。
        linq!(self.ctx.set::<Order>(), |o: Order| o.batch == batch;
            set o.checked, true;
            execute_update
        )
        .await?;

        // 2) 查询复用事务连接，能读到未提交的批量更新。
        let rows = linq!(self.ctx.set::<Order>(), |o: Order| o.batch == batch)
            .to_list()
            .await?;

        if !rows.iter().all(Order::is_valid) {
            txn.rollback().await?;
            self.ctx.clear_tracking();
            return Ok(false);
        }

        self.ctx.save_changes().await?;
        txn.commit().await?;
        Ok(true)
    }
}
```

## 5. use_transaction：闭包便利 API

不需要手动返回值时，可以用 `use_transaction`：

```rust
impl TransferHandler {
    pub async fn create_empty_account(&mut self, owner: &str) -> EFResult<()> {
        let owner = owner.to_string();
        self.ctx
            .use_transaction(move |ctx| {
                Box::pin(async move {
                    ctx.add::<Account>(Account {
                        id: 0,
                        owner,
                        balance: 0.0,
                    });
                    ctx.save_changes().await?;
                    Ok(())
                })
            })
            .await
    }
}
```

- `Ok` 提交，`Err` 回滚；
- 需要“回滚 + 返回业务结果”时，用 `begin_transaction` 手动句柄；
- 闭包内所有语句同样加入事务。

## 6. 隔离级别

```rust
let txn = ctx
    .begin_transaction_with(IsolationLevel::RepeatableRead)
    .await?;
// ...
txn.commit().await?;
```

或：

```rust
ctx.use_transaction_with(IsolationLevel::RepeatableRead, |ctx| {
    Box::pin(async move {
        // 读取账户、计算、写回
        Ok(())
    })
})
.await?;
```

| 数据库 | `*_with` 的行为 |
|--------|------------------|
| MySQL | 在 `START TRANSACTION` 之前执行 `SET TRANSACTION ISOLATION LEVEL` |
| PostgreSQL | `BEGIN` 之后、首条查询之前执行 `SET TRANSACTION` |
| SQLite | 只接受 `Serializable`；其他级别返回 `EFError::Provider` |

建议：

- 普通转账用数据库默认隔离级别即可；
- 需要“同一事务内多次读取同一行结果一致”时用 `RepeatableRead`；
- 不要为了“保险”一律 `Serializable`，它会显著增加锁冲突和重试；
- `set_isolation` 是旧句柄 API，跨库行为不完全一致，新代码优先用 `*_with`。

## 7. savepoint

`save_changes` 在事务内自动使用 savepoint；也可以手动：

```rust
let mut txn = ctx.begin_transaction().await?;
ctx.save_changes().await?;

txn.create_point("before_more").await?;
// ... 可能失败的写入 ...
txn.rollback_point("before_more").await?;

txn.commit().await?;
```

- 自动 savepoint：失败的 `save_changes` 只回滚本次保存，外层事务仍可用；
- 手动 savepoint：适合“多段操作，中间某段可丢弃”；
- `release_point` 提交 savepoint。

## 8. 错误、重试与状态

```rust
match handler.transfer(from, to, amount).await {
    Ok(TransferOutcome::Done) => { /* 成功 */ }
    Ok(TransferOutcome::InsufficientFunds) => { /* 业务拒绝，数据库已回滚 */ }
    Err(e) => {
        // 数据库/程序错误；未提交的句柄已 drop 回滚。
        eprintln!("transfer failed: {e}");
        handler.ctx.clear_tracking();
    }
}
```

- 回滚后调用 `clear_tracking()`，避免已回滚实体在后续 `save_changes` 中再次提交；
- 重试时重新查询数据，不要复用回滚前的实体；
- 不要在网络请求、文件 IO、长计算中持有事务；
- 唯一约束/死锁等错误建议在业务层做有限次退避重试，并确保操作幂等。

## 9. 测试

```rust
#[tokio::test]
async fn transfer_rolls_back_on_insufficient_funds() -> EFResult<()> {
    let mut ctx = test_context().await?;
    ctx.ensure_created().await?;

    ctx.add::<Account>(Account { id: 1, owner: "a".into(), balance: 100.0 });
    ctx.add::<Account>(Account { id: 2, owner: "b".into(), balance: 0.0 });
    ctx.save_changes().await?;

    let mut handler = TransferHandler { ctx };
    let outcome = handler.transfer(1, 2, 500.0).await?;
    assert_eq!(outcome, TransferOutcome::InsufficientFunds);

    let accounts = handler.ctx.set::<Account>().query().to_list().await?;
    assert_eq!(accounts.iter().find(|a| a.id == 1).unwrap().balance, 100.0);
    assert_eq!(accounts.iter().find(|a| a.id == 2).unwrap().balance, 0.0);
    Ok(())
}
```

真实 MySQL/PostgreSQL 行为由 CI 的 `mysql_crud_tests` / `postgres_crud_tests` 覆盖。

## 10. 常见坑

| 坑 | 正确做法 |
|----|----------|
| 在事务里用 `read` 副本查询 | 事务只在 write context；副本可能读不到未提交写 |
| 业务拒绝抛 `Err` | `txn.rollback().await?` 后返回 `Ok(result)` |
| 回滚后不清理跟踪状态 | 调用 `ctx.clear_tracking()` |
| 事务里做 HTTP/文件 IO | 先完成 IO，再开短事务 |
| 金额用 `f64` | 生产用 `Decimal` 或整数分 |
| 事务失败后复用旧实体重试 | 重新查询，或先 `clear_tracking()` |

## 11. 检查清单

- [ ] 写操作、批量 DML、`save_changes` 都在同一个 `begin_transaction` / `use_transaction`；
- [ ] 业务拒绝用 `txn.rollback()`，基础设施错误用 `Err`；
- [ ] 回滚后调用 `clear_tracking()`；
- [ ] 事务内不调用副本 `read` context；
- [ ] 事务尽量短，不在其中做网络/文件 IO；
- [ ] 需要隔离级别时使用 `begin_transaction_with` / `use_transaction_with`；
- [ ] 回滚/重试后重新查询，不复用旧跟踪实体。

下一篇：[读写分离实战：主库写入、副本查询](read-write-splitting.md)
