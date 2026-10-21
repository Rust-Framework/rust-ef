# 事务实战：转账、批量 DML 与业务回滚

本章用一个“账户转账”场景，把事务 API 串成完整业务代码，并覆盖：

- `transaction` + `TxOutcome`：回滚并返回业务结果；
- 批量 DML、查询、`save_changes` 在同一个事务里；
- 隔离级别的选择；
- 旧式 `begin_transaction` 手动句柄的边界；
- 失败、重试、测试与常见坑。

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

## 3. 用 `transaction` 表达业务回滚

```rust
impl TransferHandler {
    pub async fn transfer(
        &mut self,
        from_id: i32,
        to_id: i32,
        amount: f64,
    ) -> EFResult<TransferOutcome> {
        self.ctx
            .transaction(|ctx| {
                Box::pin(async move {
                    let mut from = ctx
                        .set::<Account>()
                        .query()
                        .find(from_id)
                        .await?
                        .ok_or_else(|| EFError::query(format!("account {from_id} not found")))?;

                    let mut to = ctx
                        .set::<Account>()
                        .query()
                        .find(to_id)
                        .await?
                        .ok_or_else(|| EFError::query(format!("account {to_id} not found")))?;

                    if from.balance < amount {
                        // 回滚，但返回成功的业务结果。
                        return Ok(TxOutcome::Rollback(TransferOutcome::InsufficientFunds));
                    }

                    from.balance -= amount;
                    to.balance += amount;
                    ctx.update::<Account>(from);
                    ctx.update::<Account>(to);

                    // 在事务内 flush：不提交，由 transaction 统一提交/回滚。
                    ctx.save_changes().await?;

                    Ok(TxOutcome::Commit(TransferOutcome::Done))
                })
            })
            .await
    }
}
```

行为：

| 闭包返回 | 数据库 | 跟踪状态 | 外层结果 |
|----------|--------|----------|----------|
| `TxOutcome::Commit(v)` | 提交 | 保留 | `Ok(v)` |
| `TxOutcome::Rollback(v)` | 回滚 | `clear_tracking()` | `Ok(v)` |
| `Err(e)` | 回滚 | `clear_tracking()` | `Err(e)` |

因此“余额不足”是正常业务分支，不需要用 `Err` 表达；真正的数据库/程序错误才用 `Err`。

## 4. 批量 DML + 查询 + save_changes

下面这个例子把“批量更新 → 读回校验 → 条件回滚”放在同一个事务：

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
        self.ctx
            .transaction(|ctx| {
                Box::pin(async move {
                    // 1) 批量 DML 加入事务，不立即提交。
                    linq!(ctx.set::<Order>(), |o: Order| o.batch == batch;
                        set o.checked, true;
                        execute_update
                    )
                    .await?;

                    // 2) 查询复用事务连接，能读到未提交的批量更新。
                    let rows = linq!(ctx.set::<Order>(), |o: Order| o.batch == batch)
                        .to_list()
                        .await?;

                    if !rows.iter().all(Order::is_valid) {
                        return Ok(TxOutcome::Rollback(false));
                    }

                    ctx.save_changes().await?;
                    Ok(TxOutcome::Commit(true))
                })
            })
            .await
    }
}
```

如果你只需要“成功提交、失败回滚”，不需要返回业务值，可以用 `use_transaction`：

```rust
self.ctx
    .use_transaction(|ctx| {
        Box::pin(async move {
            ctx.add::<Account>(Account {
                id: 0,
                owner: "new".into(),
                balance: 0.0,
            });
            ctx.save_changes().await?;
            Ok(())
        })
    })
    .await?;
```

## 5. 隔离级别

```rust
use rust_ef::prelude::*;

let outcome = self
    .ctx
    .transaction_with(IsolationLevel::RepeatableRead, |ctx| {
        Box::pin(async move {
            // 读取账户、计算、写回
            Ok(TxOutcome::Commit(()))
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

## 6. 手动句柄 `begin_transaction`

`begin_transaction` 返回一个**独立**事务句柄，保持旧语义：

```rust
use rust_ef::provider::DbValue;

let mut txn = self.ctx.begin_transaction().await?;

txn.connection()
    .execute(
        "UPDATE accounts SET balance = balance - ? WHERE id = ?",
        &[DbValue::F64(amount), DbValue::I32(from_id)],
    )
    .await?;

txn.commit().await?;
```

注意：

- 它不是 ambient transaction：查询和 `save_changes()` **不会**加入；
- `transaction_mut()` 对它返回 `None`；
- 未提交就 drop 会在后台回滚（best effort，需要 Tokio runtime）；
- 只适合“一段 raw SQL / 手动脚本”；
- 需要 `save_changes`、查询、批量 DML 一起回滚时，用 `transaction`。

## 7. 错误、重试与状态

```rust
match handler.transfer(from, to, amount).await {
    Ok(TransferOutcome::Done) => { /* 成功 */ }
    Ok(TransferOutcome::InsufficientFunds) => { /* 业务拒绝，数据库已回滚 */ }
    Err(e) => {
        // 数据库/程序错误，事务已回滚，跟踪状态已清理。
        eprintln!("transfer failed: {e}");
    }
}
```

- `transaction` 的所有回滚路径都会 `clear_tracking()`；
- 重试时重新查询数据，不要复用回滚前的实体；
- 不要在网络请求、文件 IO、长计算中持有事务；
- 唯一约束/死锁等错误建议在业务层做有限次退避重试，并确保操作幂等。

## 8. 测试

SQLite 适合做事务行为测试：

```rust
#[tokio::test]
async fn transfer_rolls_back_on_insufficient_funds() -> EFResult<()> {
    let mut ctx = test_context().await?; // 使用 use_sqlite_in_memory / 临时文件
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

真实 MySQL/PostgreSQL 行为由 CI 的 `mysql_crud_tests` / `postgres_crud_tests` 覆盖，包括 savepoint、隔离级别和回滚。

## 9. 常见坑

| 坑 | 正确做法 |
|----|----------|
| 在 `transaction` 里用 `read` 副本查询 | 事务只在 write context；副本可能读不到未提交写 |
| 用 `begin_transaction` 包住 `save_changes` 并期待一起回滚 | 用 `transaction`；`begin_transaction` 是独立句柄 |
| 余额不足抛 `Err` | 用 `TxOutcome::Rollback(BusinessResult)` |
| 事务里做 HTTP/文件 IO | 先完成 IO，再开短事务 |
| 金额用 `f64` | 生产用 `Decimal` 或整数分 |
| 事务失败后复用旧实体重试 | 重新查询，或先 `clear_tracking()` |

## 10. 检查清单

- [ ] 写操作、批量 DML、`save_changes` 都在同一个 `transaction` / `use_transaction`；
- [ ] 业务拒绝用 `TxOutcome::Rollback`，基础设施错误用 `Err`；
- [ ] 事务内不调用副本 `read` context；
- [ ] 事务尽量短，不在其中做网络/文件 IO；
- [ ] 需要隔离级别时使用 `transaction_with` / `use_transaction_with`；
- [ ] 手动 `begin_transaction` 只用于 raw SQL；
- [ ] 回滚/重试后重新查询，不复用旧跟踪实体。

下一篇：[读写分离实战：主库写入、副本查询](read-write-splitting.md)
