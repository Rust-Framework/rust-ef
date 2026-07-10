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

| 类型 | 默认可空性 | 加 `#[required]` 后 |
|------|-----------|-------------------|
| `String` | 可空 | `NOT NULL` |
| `i32` / `f64` | 非空（值类型） | 无变化 |
| `Option<T>` | 可空 | 冲突，不建议同时使用 |

## 设计要点

| 实践 | 说明 |
|------|------|
| 自增主键用 `i32` | `i64` 也支持，但 `i32` 在大多数场景足够且回填路径最成熟 |
| `Option<String>` 不加 `#[required]` | 否则语义冲突 |

下一节：[索引、唯一性与并发标记](indexes-concurrency.md)
