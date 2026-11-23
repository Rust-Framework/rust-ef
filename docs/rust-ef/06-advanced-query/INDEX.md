# 第六章 高级查询

## 本章小节

| 小节 | 内容 |
|------|------|
| [聚合函数：SUM / AVG / MIN / MAX](aggregation.md) | 聚合终端方法、类型化标量聚合与空集语义 |
| [GROUP BY 与 HAVING](group-by-having.md) | 运行时分组报表、HAVING 与 `linq!` 可选写法 |
| [类型化投影与报表行](typed-projections.md) | `SelectItem`/`SqlExpr`、别名、COALESCE/CASE、`IRow`/`FromRow` |
| [JOIN 查询](join-queries.md) | `inner_join`、`left_join` 与手动关联 |
| [CTE 与 Window 函数](cte-window-functions.md) | `with ... as` 语法糖与窗口投影 |
| [全局查询过滤器](global-query-filters.md) | `ModelBuilder::has_query_filter` 与软删除 |
| [原始 SQL 与已知限制](raw-sql-limitations.md) | `sql_query_rows`/`FromRow`、子查询限制与回避策略 |

## 下一步

从 [聚合函数](aggregation.md) 开始。
