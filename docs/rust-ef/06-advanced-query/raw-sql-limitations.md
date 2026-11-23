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

`BoolExpr::Raw` 是**框架内部**变体：`RawSql` 的字段与 `BoolExpr::raw()` 都是 `pub(crate)`，外部代码无法构造它，因此不存在"用 Raw 拼 SQL"的注入通道。

## 原始 SQL

当 `linq!` 或[类型化投影](typed-projections.md)无法表达需求时（多表 JOIN、子查询、窗口表达式……），用 provider 连接或 `ctx.sql_query*` 执行**参数化**原始 SQL：

```rust
use rust_ef::provider::DbValue;

let mut conn = ctx.provider().get_connection().await?;
let rows = conn
    .query(
        "SELECT id, name FROM blogs WHERE rating > ? ORDER BY id",
        &[DbValue::I32(5)],
    )
    .await?;

// 结果映射为实体（T: IFromRow + IEntityType）
let blogs: Vec<Blog> = ctx
    .sql_query(
        "SELECT * FROM blogs WHERE rating > ? ORDER BY id",
        &[DbValue::I32(5)],
    )
    .await?;
```

- 值一律通过 `DbValue` 参数绑定，禁止 `format!` 拼接用户输入。
- 列名/表名只能来自代码常量；请求字段决定列名/排序字段是禁止项。
- `ctx.sql_query::<T>(...)` 要求 `T: IFromRow + IEntityType`（实体）；结果映射按实体属性位置。

### 报表行：`sql_query_rows` + `IRow`

报表结果不是实体时，用 `DbContext::sql_query_rows` 把行物化为任意 `IRow`：`#[derive(FromRow)]` 结构体、1..=12 元组、或实体（实体经 blanket impl 自动实现 `IRow`）。

```rust
use rust_ef::FromRow;

#[derive(FromRow)]
struct BlogName {
    id: i32,
    name: String,
}

let names: Vec<BlogName> = ctx
    .sql_query_rows("SELECT id, name FROM blogs WHERE rating > ? ORDER BY id", &[DbValue::I32(5)])
    .await?;

// 元组目标同样可用；要求行列数完全一致
let pairs: Vec<(i32, String)> = ctx
    .sql_query_rows("SELECT id, name FROM blogs ORDER BY id", &[])
    .await?;
```

`SelectQueryBuilder::to_list_rows::<R>()` 与 `sql_query_rows` 走同一个 `IRow` 物化路径——分组投影也能直接映射到 `#[derive(FromRow)]` 结构体或元组。

**映射是位置式的**：provider 的 `query` 只返回 `Vec<Vec<DbValue>>`、不带列名，因此 `IRow` 按 `SELECT` 列顺序匹配：

- `SELECT` 列顺序必须与结构体字段声明顺序一致，列数不足/超长都是 typed `EFError::TypeConversion`（信息含字段名/列序号）。
- 单元格类型不匹配同样报带字段名/序号的 `TypeConversion`。
- 可空列用 `Option<T>` 字段/元组元素（SQL `NULL` → `None`）；裸类型遇到 `NULL` 报 `TypeConversion`。
- 给表达式加 `AS` 别名只影响结果列名（`ORDER BY` 引用），**不改变**位置映射。
- 需要“按列名映射”的场景只有实体路径：见下一节 `from_row_named`。

### 实体行物化：位置版 vs 按列名版

框架默认查询路径（`to_list()`、`ctx.sql_query::<T>()`）用**位置版** `IFromRow::from_row`：
要求结果行顺序与实体属性顺序一致（模型声明顺序，即 `SELECT *` 的列顺序）。

视图、迁移重排过的列、或手写 `SELECT` 的列顺序与模型不一致时，用**按列名版**避免静默错位：

```rust
use rust_ef::entity::materialize_entities_named;

let mut conn = ctx.provider().get_connection().await?;
// 列顺序故意与模型不同：rating, name, id
let rows = conn
    .query("SELECT rating, name, id FROM blogs ORDER BY id", &[])
    .await?;

// column_names 必须与每行的 SELECT 列顺序一致
let names: &[&str] = &["rating", "name", "id"];
let blogs: Vec<Blog> = materialize_entities_named(names, &rows)?;
```

- `materialize_entities_named(column_names, rows)` 与 `T::from_row_named(&[(name, value)])` 都按列名匹配，不要求列顺序；缺列返回 typed `EFError::TypeConversion`。
- provider 的 `query` 只返回 `Vec<Vec<DbValue>>`、不带列名，所以需要调用方提供列名数组；若 `SELECT` 列顺序与模型一致，继续用位置版即可。

## 已知限制

| 限制 | 说明 | 回避策略 |
|------|------|----------|
| **`linq!` 需显式类型** | `|b: Blog|` 不能省略 | 必须标注实体类型 |
| **类型化投影不支持子查询表达式** | `SelectItem`/`SqlExpr` 覆盖列、聚合、COALESCE、CASE（`FilterCondition`）、常量、加减；没有子查询/窗口表达式节点 | 用参数化原始 SQL + `sql_query_rows` + `#[derive(FromRow)]`；窗口投影现有 API 见 [CTE 与 Window 函数](cte-window-functions.md) |
| **`IRow` 报表行按位置映射** | provider 不返回列名；`SELECT` 顺序必须与结构体/元组顺序一致 | 列顺序可控时用投影/手写 `SELECT` 固定顺序；实体改用 `from_row_named` |
| **只读钩子不能改实体** | `on_saving` 的 `SaveChangesContext` 不含实体引用；要写服务端字段用 `on_saving_mut`（见 [SaveChanges 拦截器](../10-di-interceptors/save-changes-interceptors.md)） | 软删除仍手动标记；时间戳用 `AuditInterceptor` 或 `on_saving_mut` |
| **表达式索引/部分索引有方言差异** | MySQL 无部分索引；非唯一部分索引会被跳过 | 用 `has_index(...).has_filter(...)`，按 [索引文档](../03-entity-design/indexes-concurrency.md) 的方言矩阵验收 |
| **SQLite ALTER 能力有限** | 既有表不能增删 CHECK/FK；类型变更需重建表 | 迁移脚本会输出 `-- WARNING`；详见 [自动升级流程](../09-transactions-migrations/auto-upgrade.md) |

## 设计要点

| 实践 | 说明 |
|------|------|
| 80% 场景用 `linq!` | 保持类型安全和可维护性 |
| 20% 复杂场景用原始 SQL | 不强行用 ORM 表达一切 |
| 原始 SQL 集中管理 | 放入 `repositories/` 或 `sql/` 目录，便于审查与参数化检查 |

下一章：[变更跟踪](../07-change-tracking/INDEX.md)
