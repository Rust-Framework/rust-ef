# 原始 SQL 与已知限制

## 原始 SQL 片段

当 `linq!` 无法表达需求时，可注入原始 SQL：

```rust
use rust_ef::query::BoolExpr;

let set = ctx.set::<Blog>();
let query = set.query().filter(|qb| {
    // 无法直接用 linq! 表达时，退回到原始条件
    qb // 链式继续...
});
```

更直接的方式是通过全局过滤器或 `BoolExpr::Raw`（高级用法，需谨慎处理参数化）。

## 已知限制（v0.5）

| 限制 | 说明 | 回避策略 |
|------|------|----------|
| ~~无子查询~~ | ✅ v0.5 已实现 `any`/`none`/`all`（EXISTS/NOT EXISTS） | — |
| ~~无关联过滤~~ | ✅ v0.5 已支持基于导航元数据的子查询过滤 | — |
| ~~日期/UUID/Decimal 类型~~ | ✅ v0.5 已通过可选 feature 支持（`chrono` / `uuid` / `decimal`） | — |
| ~~无 CTE / Window 函数~~ | ✅ v1.1 已支持 `WITH` 和 `ROW_NUMBER()` 等 10 种窗口函数（见 [CTE 与 Window 函数](cte-window-functions.md)） | — |
| **linq! 需显式类型** | `|b: Blog|` 不能省略 | 必须标注实体类型 |
| ~~无 Lazy Loading~~ | ✅ v1.1 已支持 opt-in 按需加载（见 [Lazy Loading](../04-relationships/lazy-loading.md)） | — |
| **拦截器只读** | `SaveChangesContext` 不含实体引用，无法在拦截器中改字段 | 手动标记 + 拦截器审计（见软删除/审计示例） |

## 何时退回原始 SQL

```rust
// 当 ORM 无法表达时，直接使用 provider 的原始连接
let mut conn = ctx.provider().get_connection().await?;
let rows = conn.query("SELECT * FROM complex_view WHERE ...", &[]).await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 80% 场景用 `linq!` | 保持类型安全和可维护性 |
| 20% 复杂场景用原始 SQL | 不强行用 ORM 表达一切 |
| 原始 SQL 集中管理 | 放入 `repositories/` 或 `sql/` 目录，便于审查 |

下一章：[变更跟踪](../07-change-tracking/INDEX.md)
