# Add / Attach / Update / Remove

`rust-ef` 使用显式状态机跟踪实体变更。每个被跟踪的实体处于以下四种状态之一：

| 状态 | 说明 | 对应操作 |
|------|------|----------|
| `Added` | 新实体，将在 SaveChanges 时 INSERT | `ctx.add::<T>(entity)` |
| `Unchanged` | 从数据库加载，未修改 | `ctx.attach::<T>(entity)` |
| `Modified` | 属性值与快照不同 | `ctx.update::<T>(entity)` 或 `detect_changes()` |
| `Deleted` | 将在 SaveChanges 时 DELETE | `ctx.remove_at::<T>(index)` |

## Add（新增）

```rust
ctx.add::<Blog>(Blog {
    blog_id: 0,  // 自增主键占位
    url: "https://new.blog".into(),
    rating: 5,
});

ctx.save_changes().await?;  // 执行 INSERT
```

## Attach（附加）

```rust
// 从外部加载的数据，标记为 Unchanged 以便后续 DetectChanges
let blog = load_from_cache().await?;
ctx.attach::<Blog>(blog);
```

## Update（修改）

```rust
let mut blog = ctx.set::<Blog>().query().find(1).await?.unwrap();
blog.rating = 10;

ctx.update::<Blog>(blog);
ctx.save_changes().await?;  // 执行 UPDATE
```

## Remove（删除）

```rust
let blogs = ctx.set::<Blog>().query().to_list().await?;
ctx.set::<Blog>().clear_entries();

for blog in blogs {
    if blog.rating < 2 {
        ctx.attach::<Blog>(blog);
    }
}
ctx.remove_all::<Blog>();

ctx.save_changes().await?;  // 执行 DELETE
```

## 状态转换图

```
Add()        Attach()       DetectChanges()       RemoveAt()
  ↓            ↓                  ↓                   ↓
Added    Unchanged  --------->  Modified  --------->  Deleted
                  修改属性        (update 标记)       (save_changes)
```

## 设计要点

| 实践 | 说明 |
|------|------|
| `update()` 是显式的 | 修改实体后必须调用，否则 SaveChanges 不会提交 UPDATE |
| `remove_at` 按索引 | 因为 `DbSet` 内部存储为 `Vec`，按索引定位最高效 |
| 批量删除用 `ExecuteDelete` | 见 [第八章](../08-bulk-operations/INDEX.md)，避免先加载再标记 |
| `find(id)` 用 PK 元数据 | 不再硬编码 `"id"`，复合主键用 `find_by_key` |

下一节：[SaveChanges 与事务边界](save-changes.md)
