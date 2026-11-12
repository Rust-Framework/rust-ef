# 第三章 实体设计

## 本章小节

| 小节 | 内容 |
|------|------|
| [EntityType 派生与属性配置](derive-attributes.md) | `#[derive(EntityType)]` 与 20+ 属性、`#[derive(DbEnum)]` |
| [主键、自增与必填约束](keys-constraints.md) | `#[primary_key]`、复合主键、备用键、视图/无键实体、schema |
| [索引、唯一性与并发标记](indexes-concurrency.md) | `#[index]`、`#[unique]`、`#[concurrency_check]`、部分/表达式索引与 INCLUDE |
| [类型映射参考表](type-mapping.md) | Rust ↔ DbValue ↔ 三方数据库列类型、`has_column_type`/精度/定长/排序规则 |
| [EFCore 实体配置对齐矩阵](efcore-parity.md) | EntityTypeBuilder / PropertyBuilder 逐项对照与剩余边界 |

## 下一步

从 [EntityType 派生与属性配置](derive-attributes.md) 开始。
