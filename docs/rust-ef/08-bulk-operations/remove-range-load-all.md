# RemoveRange 与 LoadAll

## RemoveRange

标记多个已加载的实体为 `Deleted`：

```rust
let mut set = ctx.set::<Blog>();
let to_remove = set.query()
    .filter(linq!(|b: Blog| b.rating < 2))
    .to_list().await?;

set.remove_range(&to_remove);
ctx.save_changes().await?;
```

> `remove_range` 要求实体实现 `PartialEq`，按值匹配定位条目。

## LoadAll

将全表数据加载到 `DbSet` 中，全部标记为 `Unchanged`：

```rust
let mut set = ctx.set::<Blog>();
set.load_all().await?;  // 全部 attach 为 Unchanged

// 现在可以遍历并修改
for blog in set.tracked_entries_mut() {
    blog.rating += 1;
}
ctx.save_changes().await?;  // 自动检测所有修改
```

## 设计要点

| 实践 | 说明 |
|------|------|
| `remove_range` 适合小批量 | 大数据量直接用 `execute_delete()` |
| `load_all` 适合全表更新场景 | 如批量计算评分、同步状态 |
| 加载前 `clear_entries()` | `load_all` 内部已调用，无需手动处理 |

下一章：[事务与迁移](../09-transactions-migrations/INDEX.md)
