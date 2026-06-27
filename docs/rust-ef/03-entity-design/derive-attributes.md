# EntityType 派生与属性配置

`#[derive(EntityType)]` 是 `rust-ef` 的元数据核心。它自动为实体生成表映射、列映射、行物化和快照对比所需的所有 trait 实现。

## 完整属性示例

```rust
#[derive(Debug, Clone, EntityType)]
#[table("products")]
pub struct Product {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    #[max_length(100)]
    #[unique]
    pub sku: String,

    #[column("product_name")]
    pub name: String,

    pub price: f64,

    #[required]
    pub in_stock: bool,

    #[not_mapped]
    pub temp_discount: Option<f64>,
}
```

## 属性详解

### `#[table("name")]`

指定实体映射的数据库表名。未指定时，derive 宏会使用结构体名称的蛇形复数形式（如 `Product` → `products`）。

### `#[column("name")]`

当字段名与数据库列名不一致时使用。未指定时，字段名即为列名。

### `#[required]`

标记列非空。对 `String` 等类型尤其重要，否则默认为可空。

### `#[max_length(n)]`

对字符串类型设置最大长度，影响 DDL 生成（如 `VARCHAR(200)`）。

### `#[not_mapped]`

该字段不映射到数据库列，仅存在于内存中。常用于计算属性或临时状态。

### `#[context("key")]`（v1.1.0）

将实体标记到指定的 keyed `DbContext`。未标注时，实体归属于默认上下文（`context_key = None`）。用于多数据库场景下隔离不同上下文的实体。

```rust
#[derive(Debug, Clone, EntityType)]
#[context("logs")]
#[table("log_entries")]
pub struct LogEntry {
    #[primary_key]
    pub id: i32,
    pub message: String,
}
```

配合 `#[entity(T, "key")]` 配置和 `add_dbcontext_keyed` 使用，详见 [多数据库 Keyed 注册](../10-di-interceptors/keyed-databases.md)。

## 设计要点

| 实践 | 说明 |
|------|------|
| 始终标注 `#[primary_key]` | 没有主键的实体无法执行 `find(id)` 和 UPDATE/DELETE |
| `Clone` 必须手动派生 | `#[derive(Clone)]` 是 `EntityType` 的前置条件 |
| `Debug` 建议派生 | 便于日志和测试输出 |

下一节：[主键、自增与必填约束](keys-constraints.md)
