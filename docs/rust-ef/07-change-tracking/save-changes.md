# SaveChanges 与事务边界

`save_changes()` 是 `rust-ef` 的**工作单元（Unit of Work）**提交点。它将所有已跟踪的变更封装在一个数据库事务中。

## 事务行为

```rust
ctx.add::<Blog>(blog1);
ctx.add::<Post>(post1);
ctx.update::<Blog>(blog2);

// 以下所有操作在一个事务内执行：
// INSERT INTO blogs ...
// INSERT INTO posts ...
// UPDATE blogs SET ... WHERE ...
let result = ctx.save_changes().await?;

println!("Added: {}, Updated: {}, Deleted: {}",
    result.added, result.updated, result.deleted);
```

> 在 `use_transaction` / `transaction` 内调用 `save_changes()` 时，它只把变更 flush 到外层事务连接，**不会提交**；提交或回滚由外层事务决定。每次 flush 会创建/释放一个 savepoint，失败时回滚到该 savepoint，外层事务仍可继续。详见[手动事务](../09-transactions-migrations/manual-transactions.md)。

## 失败与回滚

任一操作失败时，事务自动回滚：

```rust
match ctx.save_changes().await {
    Ok(result) => println!("Saved: {}", result.total()),
    Err(e) => {
        // 本次 save 已回滚；如不再重试，可调用 ctx.clear_tracking() 清理跟踪状态
        eprintln!("Save failed: {e}");
    }
}
```

## 拦截器钩子

`save_changes()` 在事务前后触发拦截器：

1. `on_saving` —— 事务开始前（可验证、可中止）
2. `on_saved` —— 事务提交成功后（可记录审计日志）
3. `on_save_failed` —— 事务失败后（可记录失败原因）

## 设计要点

| 实践 | 说明 |
|------|------|
| 一个业务操作一个 SaveChanges | 避免频繁提交，也避免事务过大 |
| 不要在 SaveChanges 中间做 IO | 长事务会占用数据库连接 |
| 失败后检查状态 | 失败后如不再重试，调用 `clear_tracking()` 清理跟踪状态 |

下一节：[ChangeTracker 与 DetectChanges](change-tracker.md)
