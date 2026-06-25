# 批量删除 ExecuteDelete

`ExecuteDelete` 直接在数据库端执行 DELETE，无需加载实体。

## 基本用法

```rust
let affected = ctx
    .set::<Blog>()
    .query()
    .filter(linq!(|b: Blog| b.rating < 2))
    .execute_delete()
    .await?;

println!("Deleted {} blogs", affected);
```

## 清空全表

```rust
// ⚠️ 危险操作，无 WHERE 条件会删除全表
let affected = ctx
    .set::<LogEntry>()
    .query()
    .execute_delete()
    .await?;
```

## 与 RemoveAt 的区别

| 方式 | 机制 | 适用场景 |
|------|------|----------|
| `execute_delete()` | 数据库端 DELETE | 大批量删除，不需要加载实体 |
| `remove_at()` + `save_changes()` | 内存标记 + 事务 DELETE | 需要拦截器触发或关联级联 |

## 设计要点

| 实践 | 说明 |
|------|------|
| 删除前先 `any()` 确认 | 避免无意义的空 DELETE |
| 软删除场景用 `ExecuteUpdate` | 将 `deleted_at` 设为当前时间，而非物理删除 |

下一节：[RemoveRange 与 LoadAll](remove-range-load-all.md)
