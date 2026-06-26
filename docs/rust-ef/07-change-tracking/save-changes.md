# SaveChanges 与事务边界

`save_changes()` 是 `rust-ef` 的**工作单元（Unit of Work）**提交点。它将所有已跟踪的变更封装在一个数据库事务中。

## 事务行为

```rust
ctx.set::<Blog>().add(blog1);
ctx.set::<Post>().add(post1);
ctx.set::<Blog>().update(blog2);

// 以下所有操作在一个事务内执行：
// INSERT INTO blogs ...
// INSERT INTO posts ...
// UPDATE blogs SET ... WHERE ...
let result = ctx.save_changes().await?;

println!("Added: {}, Updated: {}, Deleted: {}",
    result.added, result.updated, result.deleted);
```

## 失败与回滚

任一操作失败时，事务自动回滚：

```rust
match ctx.save_changes().await {
    Ok(result) => println!("Saved: {}", result.total()),
    Err(EFError::Database(msg)) => {
        // 事务已回滚，所有跟踪状态保持不变
        eprintln!("Save failed: {}", msg);
    }
    Err(e) => return Err(e.into()),
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
| 失败后检查状态 | 回滚后实体仍标记为 Added/Modified，可重试或清理 |

下一节：[ChangeTracker 与 DetectChanges](change-tracker.md)
