# 类型化投影与报表行（SelectItem / SqlExpr / IRow）

`SelectItem` 把「表达式 + 别名」组合成一个 SELECT 列表项；配合 `select_items` 与 `to_list_rows`，可以直接得到「每组一行」的类型化报表，而不必把报表结果伪装成实体。

- 表达式统一为 `SqlExpr`：provider 中立，编译时用当前方言的占位符（`?` / `$N`），值始终走绑定参数，不拼 SQL 字符串。
- 结果行统一为 `IRow`：实体（自动实现）、`#[derive(FromRow)]` 结构体、1..=12 元组都可作为终端目标。

## 快速开始

```rust
use rust_ef::prelude::*;
use rust_ef::query::SelectItem;

let rows: Vec<(String, i64)> = ctx
    .set::<OrderItem>()
    .query()
    .group_by(&["category"])
    .select_items(&[
        SelectItem::column("category"),
        SelectItem::sum("amount"),
    ])
    .order_by_desc_raw("sum_amount")
    .to_list_rows::<(String, i64)>()
    .await?;
```

生成的 SQL：

```sql
SELECT category AS category, SUM(amount) AS sum_amount
FROM order_items
GROUP BY category
ORDER BY sum_amount DESC
```

## `SelectItem` 构造器

| 构造器 | 表达式 | 默认别名 |
|---|---|---|
| `SelectItem::column(c)` | `c` | 列名最后一段（`"o.amount"` → `amount`） |
| `SelectItem::sum(c)` | `SUM(c)` | `sum_<段>` |
| `SelectItem::avg(c)` | `AVG(c)` | `avg_<段>` |
| `SelectItem::min(c)` | `MIN(c)` | `min_<段>` |
| `SelectItem::max(c)` | `MAX(c)` | `max_<段>` |
| `SelectItem::count()` | `COUNT(*)` | `count` |
| `SelectItem::count_column(c)` | `COUNT(c)` | `count_<段>` |
| `SelectItem::literal(v, alias)` | `?`（绑定值） | 指定 |
| `SelectItem::expr(expr, alias)` | 任意 `SqlExpr` | 指定 |
| `.coalesce(fallback)` | `COALESCE(<原表达式>, ?)` | 保持原别名 |
| `.alias(name)` | 覆盖别名 | 指定 |

- 别名**裸输出**（不加引号），用于结果列名和 `ORDER BY` 引用；`ORDER BY` 可以引用投影别名。
  JOIN 报表里同名列（`a.id`/`b.id`）的默认别名都是 `id`，请显式 `.alias("a_id")` 区分。
- 别名不能用于 `HAVING` 条件——HAVING 用 `HavingExpr` 的聚合列（三种方言行为一致）。
- 列名原样输出（裸列名或 `table.column`），与现有查询层一致；不要传用户输入。

## `SqlExpr` 表达式

`SqlExpr` 是唯一的表达式 AST（`rust_ef::query` 导出）：

| 构造器 | 生成 SQL / 参数 |
|---|---|
| `SqlExpr::column(c)` | `c` |
| `SqlExpr::sum(c)` / `avg` / `min` / `max` | `SUM(c)` 等 |
| `SqlExpr::count_all()` | `COUNT(*)` |
| `SqlExpr::count_column(c)` | `COUNT(c)` |
| `SqlExpr::literal(v)` | `?` + 绑定值 |
| `SqlExpr::coalesce(fallback)` | `COALESCE(<expr>, ?)` |
| `SqlExpr::add(other)` / `SqlExpr::sub(other)` | `(<left> + <right>)` / `(<left> - <right>)` |
| `SqlExpr::case(arms, otherwise)` | `CASE WHEN <cond> THEN <expr> [ELSE <expr>] END` |

`CASE` 的 WHEN 条件只接受 `FilterCondition`（`col op value` / `IN` / `BETWEEN` / `IS NULL`），三方言可移植；**不支持子查询表达式**——需要子查询/窗口表达式时用 [原始 SQL + IRow](raw-sql-limitations.md)。

## 常量列（授权目录类场景）

```rust
// SELECT ?, ... ; 值仍走绑定参数，不是字符串拼接
SelectItem::literal("TENANT", "grant_scope")
```

SQL：`? AS grant_scope`；执行时绑定 `DbValue::String("TENANT")`。

## COALESCE：组内 NULL 的中性值（显式）

投影聚合不做隐式 COALESCE：`SUM`/`AVG` 在「全 NULL 组」是 SQL NULL，类型化解码会报 `EFError::TypeConversion`。需要中性值时显式声明：

```rust
.select_items(&[
    SelectItem::column("category"),
    SelectItem::sum("amount").coalesce(0i64),
])
```

SQL：`COALESCE(SUM(amount), ?) AS sum_amount`，参数为 `0`。

## CASE：按条件分类

```rust
use rust_ef::provider::DbValue;
use rust_ef::query::{CaseArm, FilterCondition, SelectItem, SqlExpr};

select_items(&[
    SelectItem::column("category"),
    SelectItem::expr(
        SqlExpr::case(
            vec![CaseArm {
                condition: FilterCondition::with_values(
                    "amount", ">", vec![DbValue::I64(500)],
                ),
                result: SqlExpr::literal("large"),
            }],
            Some(SqlExpr::literal("small")),
        ),
        "size",
    ),
])
```

SQL：`CASE WHEN amount > ? THEN ? ELSE ? END AS size`，参数顺序为 `[500, "large", "small"]`。

## 算术表达式

```rust
// (amount - discount) AS net
SelectItem::expr(
    SqlExpr::column("amount").sub(SqlExpr::column("discount")),
    "net",
)

// (views + ?) AS bumped
SelectItem::expr(
    SqlExpr::column("views").add(SqlExpr::literal(1i64)),
    "bumped",
)
```

## 报表行：`to_list_rows` / `#[derive(FromRow)]` / 元组

```rust
use rust_ef::FromRow;

#[derive(FromRow)]
struct CategoryTotal {
    category: String,
    total: i64,
}

let rows: Vec<CategoryTotal> = ctx
    .set::<OrderItem>()
    .query()
    .group_by(&["category"])
    .select_items(&[
        SelectItem::column("category"),
        SelectItem::sum("amount"),
    ])
    .order_by_raw("category")
    .to_list_rows::<CategoryTotal>()
    .await?;
```

- **位置映射**：provider 的查询接口只返回 `Vec<DbValue>`，不带列名；`#[derive(FromRow)]` 按字段声明顺序映射 `SELECT` 列顺序，两者必须一致。
- 行长度必须与字段数**完全相等**：缺列（太短）和多列（`SELECT` 与结构体漂移）都报 typed
  `EFError::TypeConversion`，信息包含字段名与列序号；单元格转换失败同样报字段名/序号。
- **可空单元格用 `Option<T>`**：SQL `NULL` 解码为 `None`（`Option<i64>`/`Option<String>`/…），
  LEFT JOIN 未命中行、可空报表列可直接映射；非空列请用裸类型，让 `NULL` 成为显式错误。
- 元组 1..=12 元素也可用：`.to_list_rows::<(String, i64)>()`；元组要求行列数完全一致。
- 实体由 `#[derive(EntityType)]` 生成 `impl IRow`（`entity::IFromRow` 是兼容别名路径），但只有列数与实体属性顺序一致时才能读取；
  报表行不需要 `EntityType`。同一个结构体不要再 derive `EntityType` + `FromRow`（两个 `IRow` 实现会冲突）。
- `SelectQueryBuilder::to_list()`（`Vec<Vec<DbValue>>`）与 `to_list_typed_1..4` 仍保留，适合无需命名结构体的简单场景。

## 原始 SQL 报表：`sql_query_rows`

复杂报表（多表 JOIN、子查询、窗口表达式）退回参数化原始 SQL，再用 `IRow` 物化：

```rust
#[derive(FromRow)]
struct SalesRow {
    category: String,
    total: i64,
}

let rows: Vec<SalesRow> = ctx
    .sql_query_rows(
        "SELECT category, SUM(amount) FROM order_items GROUP BY category ORDER BY category",
        &[],
    )
    .await?;
```

详细规则（位置映射、列序、参数绑定、实体按列名版）见 [原始 SQL 与已知限制](raw-sql-limitations.md)。

## 安全边界：标识符来自代码常量，值走参数

`SelectItem::column/sum/...` 的列名、`SelectItem::alias`、`SelectQueryBuilder::order_by` /
`order_by_desc`（以及显式 raw 拼写 `order_by_raw` / `order_by_desc_raw`）
都会作为 SQL **标识符原样输出**（与实体查询的 `order_by_raw` / `order_by_desc_raw` 运行时 API 一致）。
因此：

- 列名/别名只能来自实体 `COLUMN_*` 常量或硬编码白名单；**绝不能**直接拼接请求参数
  （需要「用户可选列/排序」时，请求值 → 白名单映射 → 常量）；
- debug 构建会对这些标识符做 `debug_assert!`（非标识符会立刻 panic，把漏网的白名单
  问题挡在开发期）；release 构建仍要求调用方遵守同一边界；
- 值一律走绑定参数：`SelectItem::literal`、`coalesce(fallback)`、`CASE` 条件值都是参数化的；
- 请求驱动的纯过滤/排序优先用编译期校验的 `linq!`，动态报表用运行时 builder 时按上面的
  白名单规则处理。

## 内省与编译产物

- `SelectQueryBuilder::compile_sql() -> EFResult<(String, Vec<DbValue>)>`：与 `QueryBuilder` 一致，单遍编译 SQL 并按文本顺序收集参数（日志、审计、SQL 形状断言）；非法查询状态返回 typed `EFError::Configuration`；
- `SelectQueryBuilder::state()`、`SelectItem::expression()` / `alias_name()`：报表运行时内省（例如动态 OpenAPI/导出适配器）；
- `QueryState::compile_sql_with::<T>(gen) -> EFResult<(String, Vec<DbValue>)>`：手写 `QueryState` 时的唯一正确入口（解析相关子查询、校验后单遍编译），不要自己用 `to_sql_with`/`all_params()` 拼参数（二者只是兼容包装）；`select_list()` 返回唯一 SELECT 形状（`SelectList`）。

## 参数顺序

投影表达式可能带绑定参数（常量、`COALESCE` 回退值、`CASE` 条件/结果），参数顺序由 `QueryState` 统一计算：

**CTE → SELECT 投影 → WHERE → HAVING → 集合运算**

因此投影里放常量/COALESCE/CASE 不会破坏 WHERE/HAVING 的参数位置，执行终端（`to_list`/`to_list_rows`）会自动带上完整参数，不要手工拼参数数组。

## 何时不要用类型化投影

| 场景 | 做法 |
|---|---|
| 结果列按**名字**映射、列顺序不固定 | 手写 SQL + 实体 `from_row_named`，或按位置自己解析 |
| 需要子查询/窗口表达式作为投影项 | 手写参数化 SQL + `sql_query_rows`（窗口投影现有 API 见 [CTE 与 Window 函数](cte-window-functions.md)） |
| 需要报表结果参与实体跟踪/保存 | 仍用实体查询；投影行是只读结果 |

下一节：[JOIN 查询](join-queries.md)
