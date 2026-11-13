# 批量更新 ExecuteUpdate

当需要按条件更新大量记录时，`ExecuteUpdate` 直接在数据库端执行 UPDATE，无需先加载实体到内存。普通场景用 `linq!` 宏的 `set` + `execute_update` 子句表达（`set b.rating, 3` 编译期解析为 `Blog::COLUMN_RATING`）；动态列或 CAS（条件更新）用公开的 `ExecuteUpdateBuilder::set_column`，二者最终生成同一条 UPDATE。

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

## 动态列 / CAS：`set_column`（公开 API）

当列需要运行时决定，或需要"校验 + 更新"的单语句 CAS（compare-and-swap / 乐观锁）时，用
`ExecuteUpdateBuilder::set_column(column, value)`。参数是实体 `COLUMN_*` 常量（`&'static str`）
与可转成 `DbValue` 的值；`execute()` 返回受影响行数，`0` 表示 WHERE 守卫已不匹配：

```rust
let affected = ctx
    .set::<Order>()
    .query()
    .filter(linq!(|o: Order| o.id == order_id && o.version == expected_version))
    .execute_update()
    .set_column(Order::COLUMN_STATUS, "paid")
    .set_column(Order::COLUMN_VERSION, expected_version + 1)
    .execute()
    .await?;

if affected == 0 {
    // 输掉竞争：重新读取后重试或返回冲突
}
```

- `set_column` 自 2.3.0 起是公开、受 semver 覆盖的入口（用于动态列/CAS；普通写路径仍推荐 `linq! set`）。
- 旧名 `set_column_internal` 保留一版 `#[deprecated]` 别名，新代码不要使用。

## 设计要点

| 实践 | 说明 |
|------|------|
| 优先用 `execute_update` 做批量修改 | 比先 `to_list()` 再逐条 `update()` + `save_changes()` 高效得多 |
| 注意不触发拦截器 | `execute_update` 绕过 ChangeTracker，拦截器不会执行 |
| `set` 子句的列字段用闭包访问 | `set b.rating, 3` 编译期解析为 `Blog::COLUMN_RATING`，避免拼写错误 |

下一节：[批量删除 ExecuteDelete](execute-delete.md)
