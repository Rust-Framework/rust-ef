# ChangeTracker 与 DetectChanges

`ChangeTracker` 负责对比实体的当前值与原始快照，自动标记 `Modified` 状态。

## 自动 DetectChanges

`save_changes()` 内部会自动调用 `detect_changes()`：

```rust
ctx.set::<Blog>().attach(blog);  // 记录快照
blog.rating = 99;                 // 修改属性

// save_changes 内部自动 detect_changes，发现差异后生成 UPDATE
ctx.save_changes().await?;
```

## 手动 DetectChanges

当你需要在 SaveChanges 之前了解哪些实体发生了变化：

```rust
ctx.detect_changes();

let modified = ctx.set::<Blog>().modified_entities();
for blog in modified {
    println!("Will update: {}", blog.url);
}
```

## 快照机制

`attach()` 时，`DbSet` 会调用 `entity.snapshot()` 保存一份 `HashMap<String, DbValue>`。`detect_changes()` 对比当前 `snapshot()` 与原始值，若不同则标记为 `Modified`。

## 设计要点

| 实践 | 说明 |
|------|------|
| 显式 `update()` 可跳过 DetectChanges | 如果你确定实体已修改，直接 `update()` 更高效 |
| 大量实体 Attach 后慎用 DetectChanges | 快照对比有 O(n) 开销 |

下一章：[批量操作](../08-bulk-operations/INDEX.md)
