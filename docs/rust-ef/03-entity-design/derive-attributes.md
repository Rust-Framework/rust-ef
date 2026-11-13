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

### `#[table("name", schema = "...", comment = "...", view)]`

指定实体映射的数据库表（或视图）名与表级选项：

| 选项 | 说明 |
|---|---|
| 第一个参数 | 表名（必填；`view` 时为视图名） |
| `schema = "..."` | PostgreSQL schema / MySQL 数据库前缀，生成 `"schema"."table"` |
| `comment = "..."` | 表注释（MySQL 内联/ALTER，PG `COMMENT ON TABLE`，SQLite 忽略） |
| `view` | 映射到已有视图，不参与建表/迁移 DDL |

```rust
#[table("log_entries", schema = "audit", comment = "audit log")]
#[table("active_users", view)]
#[table("orders")]
```

未标注 `#[table]` 时表名为空字符串（查询/DDL 不可用）；请显式声明，或用 Fluent `to_table("...")` 覆盖。schema/表名重命名等运行期覆盖使用 `entity.to_schema(...)` / `entity.to_table(...)`。

### `#[column("name")]`

当字段名与数据库列名不一致时使用。未指定时，字段名即为列名。

### `#[required]`

2.3.0 起**必填性默认由 Rust 类型推断**：`Option<T>` → 可空，其它标量（`String` / `i32` / `bool` …）→ `NOT NULL`。
`#[required]` 只用于把 `Option<T>` 提升为 `NOT NULL`（对非 `Option` 类型没有额外效果）。
需要把非 `Option` 字段显式改为可空时，用 Fluent `.is_nullable()` / `.is_optional()`（见下）。

### 必填 / 可空推断与覆盖

| 字段写法 | 生成的列 |
|---|---|
| `pub email: String` | `NOT NULL`（类型推断） |
| `pub nickname: Option<String>` | `NULL` |
| `#[required] pub tenant_id: Option<i32>` | `NOT NULL`（属性覆盖） |
| `pub score: i64` + `property_named("score").is_nullable()` | `NULL`（Fluent 覆盖） |

Fluent 覆盖写在 `IEntityTypeConfiguration` 里（与 EFCore 的 `IsRequired()` / `IsNullable()` 对齐）：

```rust
#[derive(Default)]
pub struct OrderConfig;

#[entity(Order)]
impl IEntityTypeConfiguration<Order> for OrderConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Order>) {
        // 非 Option 字段要可空：显式覆盖类型推断
        entity.property_named("note").is_nullable();
        // 等价别名：.is_optional()；反向提升：.is_required()
    }
}
```

> **升级提示（2.2 → 2.3）**：已装库不受影响，但旧快照对新模型做 diff 会把原先"可空"的非 `Option`
> 列报告为 `NULL → NOT NULL`；请重新导出 `model_snapshot.json` 并 review 首个迁移（见 CHANGELOG 2.3.0）。

### `#[max_length(n)]`

对字符串类型设置最大长度，影响 DDL 生成（如 `VARCHAR(200)`）。

### `#[foreign_key(Target)]`

标记标量外键列。除了生成 `FK_<Target>` 常量（多对多 join 查找用），现在还参与 DDL：生成指向 `Target` 主键的单列 `FOREIGN KEY`（约束名 `fk_<table>_<column>_<target>`）。复合外键用配置 API `has_foreign_key(...)` 声明，见[主键、自增与必填约束](keys-constraints.md)。

### `#[not_mapped]`

该字段不映射到数据库列，仅存在于内存中。常用于计算属性或临时状态。

### `#[context("key")]`

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

配合 `#[entity(T, "key")]` 配置和 `add_dbcontext_keyed` 使用，详见 [多数据库 Keyed 注册与读写分离](../10-di-interceptors/keyed-databases.md)。

## 表级与列级配置（对齐 EFCore）

### 表级

```rust
#[derive(Debug, Clone, EntityType)]
#[table("log_entries", schema = "audit", comment = "audit log")]
pub struct LogEntry { /* ... */ }  // → "audit"."log_entries"

#[derive(Debug, Clone, EntityType)]
#[table("active_users", view)]     // 映射到已有视图：不生成建表/迁移 DDL
pub struct ActiveUser { /* ... */ }

#[derive(Debug, Clone, EntityType)]
#[table("daily_stats")]
#[keyless]                         // 无主键（HasNoKey）
pub struct DailyStat { /* ... */ }
```

对应 Fluent API：`entity.to_schema("audit")`、`entity.to_view("active_users")`、`entity.has_no_key()`、`entity.has_comment("...")`、`entity.ignore("field")`、`entity.has_alternate_key(&[...])`。

### 列级

```rust
pub struct Product {
    #[primary_key] #[auto_increment] pub id: i32,
    #[column_type("VARCHAR(36)")] pub sku: String,          // HasColumnType
    #[precision(12, 2)] pub price: rust_decimal::Decimal,   // HasPrecision
    #[fixed_length] #[max_length(3)] pub currency: String,  // CHAR(3)
    #[collation("utf8mb4_bin")] pub code: String,           // HasCollation
    #[comment("unit price")] pub unit_price: i64,           // HasComment
    pub quantity: i32,
    #[computed("unit_price * quantity", stored)] pub total: i64, // HasComputedColumnSql
}
```

对应 Fluent API：`property_named(...).has_column_type(...)`、`.has_precision(p, s)`、`.is_fixed_length()`、`.has_collation(...)`、`.has_comment(...)`、`.has_default(...)`、`.has_computed_column_sql(...).is_stored()`、`.is_concurrency_token()`、`.is_required()` / `.is_nullable()`（别名 `.is_optional()`）。

计算列会从 `INSERT/UPDATE` 中自动排除（数据库负责计算）。

### 枚举映射：`#[derive(DbEnum)]`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, rust_ef::DbEnum)]
pub enum OrderStatus {
    #[default]
    Pending,
    #[db_value("PAID")]
    Paid,
    Cancelled,
}
```

生成 `From<OrderStatus> for DbValue`、`TryFrom<DbValue> for OrderStatus`、`as_str()`、`from_db_str()`；`#[derive(EntityType)]` 字段、`linq!` 过滤与行物化直接可用（等价于 EF Core `HasConversion<string>()`）。枚举需要同时 `derive(Default)`（行解析的兜底要求）。

## 设计要点

| 实践 | 说明 |
|------|------|
| 始终标注 `#[primary_key]` | 没有主键的实体无法执行 `find(id)` 和 UPDATE/DELETE |
| `Clone` 必须手动派生 | `#[derive(Clone)]` 是 `EntityType` 的前置条件 |
| `Debug` 建议派生 | 便于日志和测试输出 |

下一节：[主键、自增与必填约束](keys-constraints.md)
