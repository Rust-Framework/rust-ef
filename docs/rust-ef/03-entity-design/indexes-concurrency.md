# 索引、唯一性与并发标记

## 索引

```rust
#[derive(Debug, Clone, EntityType)]
#[table("articles")]
pub struct Article {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[index]
    pub category_id: i32,

    #[unique]
    pub slug: String,
}
```

当前版本的 `#[index]` 和 `#[unique]` 主要影响 **MigrationEngine 生成的 DDL**。查询执行时不会自动使用索引——这由数据库优化器决定。

## 并发标记

```rust
#[derive(Debug, Clone, EntityType)]
#[table("inventories")]
pub struct Inventory {
    #[primary_key]
    pub product_id: i32,

    pub quantity: i32,

    #[concurrency_check]
    pub row_version: i32,
}
```

`#[concurrency_check]` 标记的列在生成 UPDATE/DELETE SQL 时会追加到 WHERE 子句中：

```sql
UPDATE inventories SET quantity = ? WHERE product_id = ? AND row_version = ?
```

> ⚠️ v0.3 中乐观并发控制元数据已就绪，但完整的冲突检测（`rows_affected == 0` 时返回 `ConcurrencyConflict`）正在完善中。生产环境使用请先验证。

## 设计要点

| 实践 | 说明 |
|------|------|
| 高频过滤列加 `#[index]` | 帮助 MigrationEngine 生成合理的 DDL |
| 业务唯一键加 `#[unique]` | 如 `slug`、`email` 等 |
| 并发标记列用整数 | 便于每次更新时原子递增 |

下一章：[关系与导航](../04-relationships/INDEX.md)
