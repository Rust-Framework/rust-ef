# 批量更新 ExecuteUpdate

当需要按条件更新大量记录时，`ExecuteUpdate` 直接在数据库端执行 UPDATE，无需先加载实体到内存。

## 基本用法

```rust
let affected = ctx
    .set::<Blog>()
    .query()
    .filter(linq!(|b: Blog| b.rating < 3))
    .execute_update()
    .set_column("rating", 3)
    .execute()
    .await?;

println!("Updated {} blogs", affected);
```

## 多列更新

```rust
let affected = ctx
    .set::<Blog>()
    .query()
    .filter(linq!(|b: Blog| b.url.contains("old-domain")))
    .execute_update()
    .set_column("url", "https://new-domain.com")
    .set_column("updated_at", "2026-06-25")
    .execute()
    .await?;
```

## 生成的 SQL

```sql
UPDATE blogs SET rating = ? WHERE rating < ?
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 优先用 `ExecuteUpdate` 做批量修改 | 比先 `to_list()` 再逐条 `update()` + `save_changes()` 高效得多 |
| 注意不触发拦截器 | `ExecuteUpdate` 绕过 ChangeTracker，拦截器不会执行 |
| 列名字符串需与数据库一致 | 使用实体常量如 `Blog::COLUMN_RATING` 可减少拼写错误 |

下一节：[批量删除 ExecuteDelete](execute-delete.md)
