# CTE 与 Window 函数（v1.1）

v1.1.0 引入了对 **CTE（Common Table Expressions）** 和 **Window 函数** 的支持，通过 `linq!` 宏提供类型安全的语法糖，同时保留 raw SQL 模式以应对复杂场景。

## CTE 语法糖（推荐）

使用 `linq!` 宏的 `with ... as |e: T| ...; from name` 语法，以闭包表达式定义 CTE 主体：

```rust
let sql = linq!(
    ctx.set::<CteEmployee>();
    with high_earners as |e: CteEmployee| e.salary > 85_000;
    from high_earners
)
.to_sql();
```

生成的 SQL：

```sql
WITH high_earners AS (SELECT * FROM "cte_employees" WHERE "salary" > ?)
SELECT * FROM high_earners
```

### 复合 WHERE 条件

CTE 闭包体支持任意 `BoolExpr`，包括 `&&`、`||`、`!` 等：

```rust
let sql = linq!(
    ctx.set::<CteEmployee>();
    with eng_high as |e: CteEmployee| e.salary > 85_000 && e.dept == "Engineering";
    from eng_high
)
.to_sql();
```

### 多 CTE 组合

可定义多个 CTE 并在主查询中引用：

```rust
let sql = linq!(
    ctx.set::<CteEmployee>();
    with high_eng as |e: CteEmployee| e.dept == "Engineering" && e.salary > 85_000;
    with sales_low as |e: CteEmployee| e.dept == "Sales" && e.salary < 85_000;
    from high_eng
)
.to_sql();
```

### CTE 执行

CTE 语法糖生成的查询可直接执行：

```rust
let rows = linq!(
    ctx.set::<CteEmployee>();
    with high_earners as |e: CteEmployee| e.salary > 85_000;
    from high_earners
)
.to_list()
.await?;
```

> **PostgreSQL 注意**：typed 模式自动使用 provider 原生占位符（`$N`），确保 PostgreSQL 占位符连续正确。raw 模式使用 `?` 占位符，在 PostgreSQL 上可能不兼容——推荐使用 typed 模式。

## CTE Raw 模式

对于无法用闭包表达的复杂 CTE，可使用 raw 模式：

```rust
use rust_ef::provider::DbValue;

let rows = ctx
    .set::<CteEmployee>()
    .query()
    .with_cte_internal(
        "high_earners",
        "SELECT * FROM cte_employees WHERE salary > ?",
        vec![DbValue::Integer(85_000)],
        vec!["emp_id", "name", "dept", "salary"],
    )
    .from_cte("high_earners")
    .to_list()
    .await?;
```

> **限制**：raw 模式的预编译 SQL 使用 `?` 占位符，在 PostgreSQL 上不会转换为 `$N`。生产环境推荐使用 typed 模式（`linq!(with ...)` 语法糖）。

## Window 函数

`linq!` 宏的 `window` 子句支持 10 种窗口函数：

| 函数 | 说明 | SQL |
|------|------|-----|
| `row_number` | 行号 | `ROW_NUMBER()` |
| `rank` | 排名（同值跳号） | `RANK()` |
| `dense_rank` | 稠密排名（同值不跳号） | `DENSE_RANK()` |
| `lag` | 前一行值 | `LAG(...)` |
| `lead` | 后一行值 | `LEAD(...)` |
| `sum` | 累计求和 | `SUM(...)` |
| `count` | 计数 | `COUNT(...)` |
| `avg` | 平均值 | `AVG(...)` |
| `min` | 最小值 | `MIN(...)` |
| `max` | 最大值 | `MAX(...)` |

### 基本语法

```
window <func> [<field>] partition_by <expr> order_by <expr> [asc|desc] as <alias>
```

### ROW_NUMBER 示例

按部门分区，按薪资降序排名：

```rust
let sql = linq!(
    ctx.set::<WinEmployee>(),
    |e: WinEmployee| e.emp_id > 0;
    window row_number partition_by e.dept order_by e.salary desc as rn
)
.to_sql();
```

生成的 SQL：

```sql
SELECT *, ROW_NUMBER() OVER (PARTITION BY "dept" ORDER BY "salary" DESC) AS "rn"
FROM "win_employees"
WHERE "emp_id" > ?
```

### SUM 窗口聚合

计算每个部门的薪资总额：

```rust
let sql = linq!(
    ctx.set::<WinEmployee>();
    window sum e.salary partition_by e.dept as dept_total
)
.to_sql();
```

### 执行 Window 查询

```rust
let rows = linq!(
    ctx.set::<WinEmployee>();
    window row_number partition_by e.dept order_by e.salary desc as rn
)
.to_list()
.await?;
```

> **注意**：Window 函数投影列通过 `SELECT *` 附加在结果集中，`from_row` 仅读取实体字段，窗口函数列被忽略。如需读取窗口函数结果，使用 `select` 子句获取原始行。

## IN / NOT IN 子查询

v1.1.0 同时支持 `IN` / `NOT IN` 标量子查询，通过 `in_subquery` 语法表达：

```rust
// 查询有文章的 Blog
let blogs = linq!(
    ctx.set::<Blog>(),
    |b: Blog| b.blog_id.in_subquery(|p: Post| p.blog_id)
)
.to_list()
.await?;
```

生成的 SQL：

```sql
SELECT * FROM "blogs"
WHERE "blog_id" IN (SELECT "blog_id" FROM "posts")
```

### NOT IN 子查询

```rust
// 查询没有文章的 Blog
let blogs = linq!(
    ctx.set::<Blog>(),
    |b: Blog| !b.blog_id.in_subquery(|p: Post| p.blog_id)
)
.to_list()
.await?;
```

### 与其他条件组合

```rust
let blogs = linq!(
    ctx.set::<Blog>(),
    |b: Blog| b.rating > 3 && b.blog_id.in_subquery(|p: Post| p.blog_id)
)
.to_list()
.await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| **优先使用 typed 模式** | `linq!(with ...)` 语法糖自动处理占位符方言，三库通用 |
| **raw 模式仅用于复杂场景** | 闭包无法表达的 CTE 才用 `with_cte_internal` |
| **Window 列不被 from_row 读取** | 需要窗口函数结果时用 `select` 获取原始行 |
| **PostgreSQL 占位符** | typed 模式自动用 `$N`，raw 模式用 `?`（PG 不兼容） |
| **SQLite 版本要求** | CTE 需 3.8.3+，Window 函数需 3.25+ |

## 限制

- **raw CTE PostgreSQL 兼容性**：`with_cte_internal()` 的 SQL 使用 `?` 占位符，在 PostgreSQL 上不转换为 `$N`。推荐 typed 模式
- **Window 投影列读取**：`from_row` 忽略窗口函数列，需用 `select` 子句获取原始行
- **CTE 嵌套**：当前不支持 CTE 内嵌套 CTE，每个 `with` 子句独立编译

上一节：[JOIN 查询](join-queries.md)

下一节：[全局查询过滤器](global-query-filters.md)
