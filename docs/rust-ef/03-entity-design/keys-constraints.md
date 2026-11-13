# 主键、自增与必填约束

## 单一主键

```rust
#[derive(Debug, Clone, EntityType)]
#[table("users")]
pub struct User {
    #[primary_key]
    #[auto_increment]
    pub user_id: i32,

    #[required]
    pub email: String,
}
```

## 复合主键

`rust-ef` 支持复合主键，但需在 `entity_meta()` 中正确声明多列。当前 derive 宏对复合主键的自动化支持有限，复杂场景建议手动实现 `IGetKeyValues`。

```rust
impl IGetKeyValues for OrderLine {
    fn key_values(&self) -> HashMap<String, DbValue> {
        let mut m = HashMap::new();
        m.insert("order_id".into(), DbValue::I32(self.order_id));
        m.insert("line_no".into(), DbValue::I32(self.line_no));
        m
    }
}
```

## 自增回填

INSERT 后，`save_changes()` 会自动通过 `RETURNING` 语句回填自增主键值：

```rust
ctx.add::<Blog>(Blog { blog_id: 0, .. });
ctx.save_changes().await?;
// blog_id 现在包含数据库生成的实际值
```

## 必填约束

自 2.3.0 起，必填性默认**按 Rust 类型推断**（EF-12）；`#[required]` 只用于把 `Option<T>` 提升为 `NOT NULL`：

| 类型 | 默认可空性 | 加 `#[required]` 后 | 显式覆盖 |
|------|-----------|-------------------|----------|
| `String` | `NOT NULL`（非 `Option`） | 无变化 | `.is_nullable()` 改为可空 |
| `i32` / `f64` 等标量 | `NOT NULL` | 无变化 | `.is_nullable()` 改为可空 |
| `Option<T>` | 可空 | `NOT NULL`（冲突写法，不建议） | `.is_required()` 同义 |

Fluent 覆盖写在 `IEntityTypeConfiguration` 中：

```rust
#[derive(Default)]
pub struct BlogConfig;

#[entity(Blog)]
impl IEntityTypeConfiguration<Blog> for BlogConfig {
    fn configure(&self, entity: &mut EntityTypeBuilder<'_, Blog>) {
        entity.property_named("summary").is_nullable(); // 非 Option 字段改为可空
    }
}
```

## 外键（标量与复合）

标量字段的 `#[foreign_key(Target)]` 现在同时参与 DDL：生成指向 `Target` 主键的 `FOREIGN KEY` 约束（约束名 `fk_<table>_<column>_<target>`）：

```rust
#[derive(Debug, Clone, EntityType)]
#[table("order_records")]
pub struct OrderRecord {
    #[primary_key]
    #[auto_increment]
    pub id: i32,
    pub tenant_id: i32,
    #[foreign_key(Customer)]   // → FOREIGN KEY (customer_id) REFERENCES customers (id)
    pub customer_id: i32,
    // ...
}
```

复合外键在配置中声明：

```rust
entity.has_foreign_key(
    &[Voucher::COLUMN_TENANT_ID, Voucher::COLUMN_ORDER_ID],
    OrderRecord::TABLE,
    &[OrderRecord::COLUMN_TENANT_ID, OrderRecord::COLUMN_ID],
).on_delete(DeleteBehavior::Cascade);
```

- PostgreSQL / MySQL：所有表创建完后用 `ALTER TABLE ... ADD CONSTRAINT` 添加（避免建表顺序问题）。
- SQLite 不支持 `ALTER TABLE ADD CONSTRAINT`：迁移脚本在 `CREATE TABLE` 内联全部外键；`ensure_created` 只建表、不生成外键。
- 列重命名（`property_named(...).has_column_name(...)`）会自动反映到索引/外键的依赖列。

## CHECK 约束

```rust
entity.property_named("amount").has_check("amount > 0");
entity.property_named("status").has_check("status IN ('PENDING','PAID','CANCELLED')");
// 跨列约束用表级 API
entity.has_check("CK_order_total", "total = unit_price * quantity");
entity.has_check(
    "CK_ledger_succeeded",
    "status <> 'SUCCEEDED' OR (evidence IS NOT NULL AND done_at IS NOT NULL)",
);
```

- 列级约束名自动为 `CK_<table>_<column>`；表级约束名由调用方指定。
- PostgreSQL / MySQL 8.0.16+ 在建表时内联 `CONSTRAINT ... CHECK (...)`；MySQL 8.0.16 之前只解析不执行。
- SQLite 建表时内联；对既有表增删 CHECK 需要重建表，迁移引擎会输出 `-- WARNING` 注释而不是非法 SQL。

## 备用键、视图与无键实体

```rust
// HasAlternateKey：唯一约束，可作为外键引用目标
entity.has_alternate_key(&[User::COLUMN_TENANT_ID, User::COLUMN_EMAIL]);

// ToView：映射到已有视图，不生成建表/迁移 DDL
#[derive(Debug, Clone, EntityType)]
#[table("active_users", view)]
pub struct ActiveUser { /* ... */ }

// HasNoKey：无主键（投影/统计）
#[derive(Debug, Clone, EntityType)]
#[table("daily_stats")]
#[keyless]
pub struct DailyStat { /* ... */ }
```

视图与无键实体仍可查询；不要对它们调用 `find` / `remove`（依赖主键）。

## Schema（PostgreSQL schema / MySQL 数据库前缀）

```rust
#[derive(Debug, Clone, EntityType)]
#[table("log_entries", schema = "audit")]
pub struct LogEntry { /* ... */ }   // → "audit"."log_entries"
```

Fluent 等价：`entity.to_schema("audit")`。使用派生属性时 `TABLE` 常量也带 schema，导航/懒加载的 `related_table` 同步正确；仅用 Fluent `to_schema` 时跨 schema 导航需要显式处理。

## 列默认值

```rust
entity.property_named("version").has_default("1");
entity.property_named("created_at").has_default("CURRENT_TIMESTAMP");
entity.property_named("status").has_default("'PENDING'"); // 字符串字面量需自带引号
```

默认值是**原始 SQL 表达式**，生成 `... NOT NULL DEFAULT 1`；`ALTER COLUMN` 也会同步 `SET DEFAULT` / `DROP DEFAULT`（PostgreSQL）或 `MODIFY COLUMN`（MySQL）。

## 设计要点

| 实践 | 说明 |
|------|------|
| 自增主键用 `i32` | `i64` 也支持，但 `i32` 在大多数场景足够且回填路径最成熟 |
| `Option<String>` 不加 `#[required]` | 否则语义冲突 |

下一节：[索引、唯一性与并发标记](indexes-concurrency.md)
