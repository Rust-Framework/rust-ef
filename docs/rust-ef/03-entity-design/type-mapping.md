# 类型映射参考表

`rust-ef` 通过 `DbValue` 枚举承载参数值，通过 `MigrationDialect::map_column_type` 生成 DDL 列类型。本文档汇总 Rust 类型 → `DbValue` 变体 → 三方数据库列类型的对应关系。

## 内置类型映射

以下类型无需开启可选 feature，开箱即用。

| Rust 类型 | `DbValue` 变体 | PostgreSQL | MySQL | SQLite |
|-----------|----------------|:----------:|:-----:|:------:|
| `i16` | `I16` | SMALLINT | SMALLINT | INTEGER |
| `i32` | `I32` | INTEGER | INT | INTEGER |
| `i64` | `I64` | BIGINT | BIGINT | INTEGER |
| `f32` | `F32` | REAL | FLOAT | REAL |
| `f64` | `F64` | DOUBLE PRECISION | DOUBLE | REAL |
| `bool` | `Bool` | BOOLEAN | BOOLEAN | INTEGER |
| `String` | `String` | VARCHAR(n) / TEXT | VARCHAR(n) / TEXT | TEXT |
| `Vec<u8>` | `Bytes` | BYTEA | BLOB | BLOB |
| `Option<T>` | 同 `T` 或 `Null` | 同 `T`（可空） | 同 `T`（可空） | 同 `T`（可空） |

### 自增主键

`#[primary_key] #[auto_increment]` 的列在 DDL 生成时按方言特化：

| Rust 类型 | PostgreSQL | MySQL | SQLite |
|-----------|:----------:|:-----:|:------:|
| `i32` | SERIAL | INT AUTO_INCREMENT | INTEGER |
| `i64` | BIGSERIAL | BIGINT AUTO_INCREMENT | INTEGER |

## 可选 Feature 类型映射

chrono / uuid / decimal 通过可选 feature 启用。开启后，`DbValue::String` 变体承载其文本表示，避免破坏既有 ABI。

### 启用方式

```toml
[dependencies]
rust-ef = { version = "1.1", features = ["chrono", "uuid", "decimal"] }
```

Feature 通过 `rust-ef-macros` 透传到派生宏，宏根据 `cfg!(feature = "...")` 生成对应的 `from_row` 解析分支。

### chrono（`feature = "chrono"`）

| Rust 类型 | 文本格式 | PostgreSQL | MySQL | SQLite |
|-----------|----------|:----------:|:-----:|:------:|
| `chrono::DateTime<chrono::Utc>` | RFC3339（`2026-06-26T14:00:00+00:00`） | TIMESTAMPTZ | DATETIME | TEXT |
| `chrono::NaiveDateTime` | `2026-06-26 14:00:00` | TIMESTAMP | DATETIME | TEXT |
| `chrono::NaiveDate` | `2026-06-26` | DATE | DATE | TEXT |

**注意**：`NaiveDateTime::to_string()` 使用空格分隔符，`from_row` 解析使用 `parse_from_str("%Y-%m-%d %H:%M:%S%.f")`；不可改用 `FromStr`（其期望 'T' 分隔符）。匹配顺序上 `NaiveDateTime` / `NaiveDate` 必须在 `DateTime` 之前判断，避免子串误匹配。

### uuid（`feature = "uuid"`）

| Rust 类型 | 文本格式 | PostgreSQL | MySQL | SQLite |
|-----------|----------|:----------:|:-----:|:------:|
| `uuid::Uuid` | 连字符标准形式（`550e8400-e29b-41d4-a716-446655440000`） | UUID | CHAR(36) | TEXT |

依赖启用 `v4` feature，便于 `Uuid::new_v4()` 生成。

### decimal（`feature = "decimal"`）

| Rust 类型 | 文本格式 | PostgreSQL | MySQL | SQLite |
|-----------|----------|:----------:|:-----:|:------:|
| `rust_decimal::Decimal` | `to_string()`（如 `19.99`） | NUMERIC | DECIMAL(38,18) | TEXT |

MySQL 使用 `DECIMAL(38,18)` 以容纳 `rust_decimal` 的最大精度（96 位 / 38 位有效数字）。

## from_row 解析行为

派生宏为每个标量字段生成解析表达式。对于 chrono / uuid / decimal 类型，解析失败时返回 `Default::default()`（如 `DateTime` → UNIX 纪元、`Uuid` → nil UUID、`Decimal` → 0），不抛出错误。这与内置数值类型的 `unwrap_or_default` 行为一致。

| 类型 | 解析方式 |
|------|----------|
| 数值 / bool / String | `parse()` 或 `as` 转换 |
| `Vec<u8>` | hex 解码 |
| `NaiveDateTime` | `parse_from_str("%Y-%m-%d %H:%M:%S%.f")` |
| `NaiveDate` | `parse::<NaiveDate>()` |
| `DateTime<Utc>` | `parse_from_rfc3339()` + `with_timezone(&Utc)` |
| `Uuid` | `parse::<Uuid>()` |
| `Decimal` | `parse::<Decimal>()` |

## 已知限制

- **经 `String` 中转**：chrono / uuid / decimal 的 `From` impl 将其转为 `DbValue::String`，Provider 参数绑定走文本通道，未利用 PostgreSQL 原生 `TIMESTAMPTZ` / `UUID` 参数类型。后续优化项：在 Provider 层增加原生类型绑定。
- **`bool` 在 SQLite**：存储为 `"0"` / `"1"` 字符串，`from_row` 通过 `match` 处理（`"true" | "1" => true`），不可使用 `parse::<bool>()`（仅接受 `"true"` / `"false"`）。
- **解析失败静默回退**：`unwrap_or_default()` 不报错，错误数据会变成默认值。生产环境如需严格校验，应在应用层做额外验证。

## 完整示例

```rust
#[derive(Debug, Clone, EntityType)]
#[table("transactions")]
pub struct Transaction {
    #[primary_key]
    #[auto_increment]
    pub id: i32,

    #[required]
    pub reference_id: uuid::Uuid,

    pub amount: rust_decimal::Decimal,

    pub created_at: chrono::DateTime<chrono::Utc>,

    pub processed_at: chrono::NaiveDateTime,

    pub transaction_date: chrono::NaiveDate,
}
```

对应 DDL（PostgreSQL）：

```sql
CREATE TABLE "transactions" (
    "id" SERIAL NOT NULL,
    "reference_id" UUID NOT NULL,
    "amount" NUMERIC,
    "created_at" TIMESTAMPTZ,
    "processed_at" TIMESTAMP,
    "transaction_date" DATE,
    CONSTRAINT "pk_transactions" PRIMARY KEY ("id")
);
```

## 测试覆盖

`crates/core/tests/extended_types_tests.rs`（6 个测试，`#![cfg(all(feature = "chrono", feature = "uuid", feature = "decimal"))]` 门控）：

- `chrono_uuid_decimal_round_trip` — 单行插入 + 查询回读
- `multiple_transactions_query` — 多行 + 过滤
- `datetime_filter_query` — `linq!` 对 `DateTime` 字段过滤
- `update_with_chrono_fields` — UPDATE 含 chrono 字段
- `map_column_type_chrono_uuid_decimal` — DDL 类型映射断言
- `map_column_type_existing_types_still_match` — 回归：内置类型映射未被破坏

运行：

```bash
cargo test -p rust-ef --features chrono,uuid,decimal --test extended_types_tests
```

---

上一节：[索引、唯一性与并发标记](indexes-concurrency.md)  
下一章：[关系映射](../04-relationships/INDEX.md)
