# 批量更新 ExecuteUpdate

当需要按条件更新大量记录时，`ExecuteUpdate` 直接在数据库端执行 UPDATE，无需先加载实体到内存。通过 `linq!` 宏的 `set` + `execute_update` 子句表达。字符串 API（`set_column("col", val)`）已移除。

## 基本用法

```rust
// linq! 多子句形式：过滤 + set + execute_update
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.rating < 3;
    set b.rating, 3;
    execute_update
).await?;

println!("Updated {} blogs", affected);
```

## 多列更新

```rust
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.url.contains("old-domain");
    set b.url, "https://new-domain.com";
    set b.rating, 5;
    execute_update
).await?;
```

## 生成的 SQL

```sql
UPDATE blogs SET rating = ? WHERE rating < ?
```

多个 `set` 子句生成 `SET col1 = ?, col2 = ?`。

## 设计要点

| 实践 | 说明 |
|------|------|
| 优先用 `execute_update` 做批量修改 | 比先 `to_list()` 再逐条 `update()` + `save_changes()` 高效得多 |
| 注意不触发拦截器 | `execute_update` 绕过 ChangeTracker，拦截器不会执行 |
| `set` 子句的列字段用闭包访问 | `set b.rating, 3` 编译期解析为 `Blog::COLUMN_RATING`，避免拼写错误 |

下一节：[批量删除 ExecuteDelete](execute-delete.md)
