# 原始 SQL 与已知限制

## 条件无法用 `linq!` 表达时

条件层只有两个出口，值始终走绑定参数：

```rust
use rust_ef::query::{BoolExpr, FilterCondition};

// 1) 自包含条件：手写列名（编译期不校验列名，写错只会在查询执行时失败）
let expr = BoolExpr::Filter(FilterCondition::with_values(
    "rating", ">", vec![DbValue::F64(0.5)],
));
let blogs = ctx.set::<Blog>().query().filter(expr).to_list().await?;

// 2) 一个查询里混用谓词与手写条件
let blogs = ctx
    .set::<Blog>()
    .query()
    .filter(linq!(|b: Blog| b.active))
    .filter(expr)
    .to_list()
    .await?;
```

`BoolExpr::Raw` 是**框架内部**变体：`RawSql` 的字段与 `BoolExpr::raw()` 都是 `pub(crate)`，外部代码无法构造它，因此不存在"用 Raw 拼 SQL"的注入通道。需要原始 SQL 时走下一节的 provider 连接。

## 已知限制

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
