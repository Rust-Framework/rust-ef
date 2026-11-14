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

## 复合索引、唯一索引、部分索引与表达式索引

单列 `#[index]` / `#[unique]` 之外，可在 `#[entity(T)]` 配置里声明完整的索引：

```rust
#[derive(Default)]
pub struct OrderConfig;

#[entity(Order)]
impl IEntityTypeConfiguration<Order> for OrderConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Order>) {
        // 复合唯一约束：UNIQUE(tenant_id, request_key)
        entity
            .has_index(&[Order::COLUMN_TENANT_ID, Order::COLUMN_REQUEST_KEY])
            .is_unique();

        // 多列普通索引 + 显式名字
        entity
            .has_index(&[Order::COLUMN_TENANT_ID, Order::COLUMN_STATUS])
            .has_name("ix_orders_tenant_status");

        // 部分唯一索引（PostgreSQL / SQLite：... WHERE status = 'PENDING'）
        entity
            .has_index(&[Invite::COLUMN_TENANT_ID, Invite::COLUMN_CODE])
            .is_unique()
            .has_filter(linq!(|i: Invite| i.status == "PENDING"));

        // 表达式/函数索引：UNIQUE (coalesce(tenant_id, 0))
        entity
            .has_index_expression(&["coalesce(tenant_id, 0)"])
            .is_unique()
            .has_name("ux_orders_coalesce_tenant");
    }
}
```

语义与方言差异：

| API | 生成 SQL | 说明 |
|-----|----------|------|
| `.has_index(&[a, b])` | `CREATE [UNIQUE] INDEX ... (a, b)` | 一个复合索引（不再退化为两个单列索引）；默认名 `ix_<table>_<cols>`，可用 `.has_name(...)` 覆盖 |
| `.has_filter(predicate)` | PG/SQLite：`... WHERE <predicate>` | PostgreSQL / SQLite 原生部分索引 |
| `.has_filter(predicate)` + `.is_unique()`（MySQL） | `CREATE UNIQUE INDEX ... ((CASE WHEN <predicate> THEN col END), ...)` | MySQL 没有部分索引，用函数索引模拟：不匹配的行键为 NULL，UNIQUE 不比较 NULL |
| `.has_filter(predicate)`（MySQL，非唯一） | `-- WARNING ... skipped` 注释 | MySQL 无法模拟部分普通索引，跳过而不是生成错误 SQL |
| `.has_index_expression(&["coalesce(...)"])` | `CREATE INDEX ... ((coalesce(...)))` | 表达式按原样输出（方言敏感）：请使用可移植的小写 SQL |
| `.include(&[cols])` | PostgreSQL：`... INCLUDE (cols)` | MySQL/SQLite 不支持 INCLUDE，输出 warning 并只建键列索引 |
| `entity.has_alternate_key(&[cols])` | `CREATE UNIQUE INDEX ak_<table>_<cols>` | 等价 EF Core `HasAlternateKey`，可作为外键引用目标 |

> 部分索引的谓词来自 `linq!(|x: T| ...)`，值在 DDL 中会渲染成字面量；只支持简单比较与 `AND`/`OR`/`NOT`（子查询在部分索引谓词中没有实用价值）。

### 混合列 + 表达式索引（2.5.0）

`has_index_expression` 的列表可以同时包含列名和表达式；**裸标识符按列处理**，含函数/运算符的按表达式处理：

```rust
entity
    .has_index_expression(&[
        "tenant_id",                  // → `tenant_id`（列）
        "member_id",
        "role_id",
        "scope",
        "coalesce(channel_id, '')",   // → (coalesce(...))（表达式）
    ])
    .is_unique();
```

生成 MySQL 时为 `CREATE UNIQUE INDEX ... (`tenant_id`, ..., (coalesce(channel_id, '')))`，
不再把普通列包成 `(tenant_id)`（MySQL 8 会报 3762 `Functional index on a column is not supported`）。
PostgreSQL/SQLite 同样保留“列不加括号、表达式加括号”的形状。

自动索引名（`ix_<table>_<cols>`、`fk_<table>_<cols>_<target>`）统一按**可移植标识符上限 63 字节**
截断并追加稳定哈希：PostgreSQL 会静默截断更长的名字，MySQL 直接报 1059；显式 `.has_name(...)` 优先。
同一个模型重复生成的名字稳定，不会因截断碰撞。

## 表级存储选项（MySQL）

```rust
entity.engine("InnoDB").charset("utf8mb4").collate("utf8mb4_bin");
```

MySQL 生成 `) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin;`；即使不配置也会显式输出 `ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`，避免继承实例默认（如 latin1）。PostgreSQL / SQLite 忽略这些选项。

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

`rows_affected == 0` 时返回 `EFError::ConcurrencyConflict`；冲突处理策略由调用方决定（不要静默覆盖重试）。

## 设计要点

| 实践 | 说明 |
|------|------|
| 高频过滤列加 `#[index]` | 帮助 MigrationEngine 生成合理的 DDL |
| 业务唯一键加 `#[unique]` | 如 `slug`、`email` 等 |
| 并发标记列用整数 | 便于每次更新时原子递增 |

下一章：[关系与导航](../04-relationships/INDEX.md)
