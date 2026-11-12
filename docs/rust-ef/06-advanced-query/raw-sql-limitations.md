# 原始 SQL 与已知限制

## 原始 SQL

当 `linq!` 无法表达需求时，用 provider 连接执行**参数化**原始 SQL：

```rust
use rust_ef::provider::DbValue;

let mut conn = ctx.provider().get_connection().await?;
let rows = conn
    .query(
        "SELECT id, name FROM blogs WHERE rating > ? ORDER BY id",
        &[DbValue::I32(5)],
    )
    .await?;
```

- 值一律通过 `DbValue` 参数绑定，禁止 `format!` 拼接用户输入。
- 列名/表名只能来自代码常量；请求字段决定列名/排序字段是禁止项。
- 需要映射为实体时用 `ctx.sql_query::<T>(...)`（`T: IFromRow`）。

## 已知限制

| 限制 | 说明 | 回避策略 |
|------|------|----------|
| **`linq!` 需显式类型** | `|b: Blog|` 不能省略 | 必须标注实体类型 |
| **拦截器只读** | `SaveChangesContext` 不含实体引用，无法在拦截器中改字段 | 手动标记 + 拦截器审计（见软删除/审计示例） |
| **表达式索引/部分索引有方言差异** | MySQL 无部分索引；非唯一部分索引会被跳过 | 用 `has_index(...).has_filter(...)`，按 [索引文档](../03-entity-design/indexes-concurrency.md) 的方言矩阵验收 |
| **SQLite ALTER 能力有限** | 既有表不能增删 CHECK/FK；类型变更需重建表 | 迁移脚本会输出 `-- WARNING`；详见 [自动升级流程](../09-transactions-migrations/auto-upgrade.md) |

## 设计要点

| 实践 | 说明 |
|------|------|
| 80% 场景用 `linq!` | 保持类型安全和可维护性 |
| 20% 复杂场景用原始 SQL | 不强行用 ORM 表达一切 |
| 原始 SQL 集中管理 | 放入 `repositories/` 或 `sql/` 目录，便于审查与参数化检查 |

下一章：[变更跟踪](../07-change-tracking/INDEX.md)
