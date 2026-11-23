# 查询管线架构（2.8）

> 面向框架贡献者与高级使用者：说明 rust-ef 2.8 查询层的分层、编译不变量与扩展点，
> 以及为什么这些约束能让"生成 SQL / 绑定参数 / 物化结果"三者不漂移。

## 1. 分层

```text
类型化入口 (linq!)          运行时入口 (builder)
      │                            │
      └──────── Predicate<T> ──────┘       业务条件（类型安全）
                     │
              BoolExpr / HavingExpr         布尔代数（AND/OR/NOT/子查询/比较）
                     │   ├── FilterCondition（列 op 参数）
                     │   └── Compare(SqlExpr, op, SqlExpr)（HAVING/高级比较）
                     │
                  SqlExpr                   标量表达式（列/字面量/聚合/COALESCE/CASE/算术）
                     │
                QueryState                  单查询意图（FROM/JOIN/WHERE/GROUP/HAVING/ORDER/分页/CTE/集合运算/SELECT）
                     │
        try_compile_with(gen) / compile_sql()  单遍编译：边生成占位符边收集参数
                     │
        EFResult<(String, Vec<DbValue>)>   校验后交给 provider 执行
```

- **一个布尔代数**：WHERE、HAVING、CTE 谓词、子查询谓词、DML 过滤都编译为
  `BoolExpr`。`HavingExpr` 是面向 `linq!(having ...)` 的构造器，通过
  `into_bool_expr()` 降到 `BoolExpr`，不会再出现"HAVING 参数顺序单独维护"的第二套实现。
- **一个标量代数**：SELECT 投影、HAVING 比较、SET 表达式、CASE 分支共用 `SqlExpr` 的
  编译与取值逻辑。`ColumnExpr`（SET）是受限于算术子集的构造器，编译时降为 `SqlExpr`。
- **一个 SELECT 形状**：`QueryState::select_list() -> &SelectList`，
  `SelectList` 是 `Star | Columns | Projections | Aggregate | Count | Exists` 的单一状态，
  取代过去六个可同时为真的布尔/字段；非法组合由 `validate()` 在终端前一次性拒绝。

## 2. 编译不变量

1. **单遍、同序**：`QueryState::try_compile_with(gen)` 沿 SQL 文本顺序（CTE → SELECT →
   WHERE → HAVING → 集合运算）生成占位符，并在同一遍历里把值 push 进参数向量。
   `to_sql_with()` / `all_params()` 只是兼容包装；内置终端只走单遍入口。
2. **provider 语义**：占位符由 `ISqlGenerator` 决定（`?` 或 `$N`）；预编译片段
   （raw CTE、集合运算操作数）通过 `emit_raw_fragment` 适配 provider：`?` 逐个替换为
   provider 占位符，`$N` 片段按已发射参数重编号。
3. **校验前移**：非法查询（`SELECT * + GROUP BY`、`COUNT(*) + DISTINCT`、重复投影别名、
   `from_cte` 晚于 JOIN、`filter_joined` 子查询）返回 `EFError::Configuration`，
   而不是生成不可执行 SQL。错误通道由 `QueryState::try_compile_with(gen)`、
   `compile_sql_with::<T>(gen)` 以及各 builder 终端的 `compile_sql()` 提供（均返回
   `EFResult<(String, Vec<DbValue>)>`）；`to_sql_with` / `to_sql` 仅用于形状/日志
   诊断（不保证可执行）。
4. **物化与查询解耦**：`IRow`（`from_row(&[DbValue])`）是唯一行物化 trait，
   实体、`#[derive(FromRow)]` 结构体、1..=12 元组都实现它；`sql_query` 额外要求
   `IEntityType`，`sql_query_rows` 接受任意 `IRow`。
   列宽契约：**报表行严格**（`FromRow`/元组必须与 SELECT 列数完全一致，缺列/多列
   都是 typed error）；**实体按前缀映射**（`SELECT * FROM t JOIN ...` 会在实体列之后
   追加 join 列，实体忽略尾列，但缺列仍报错）。按列名的 `from_row_named` 忽略多余
   键、缺少实体列仍报错。这是 2.x 的刻意取舍，避免 JOIN 查询无法物化实体。

## 3. 类型化入口与运行时入口的边界

| 场景 | 推荐入口 | 说明 |
|---|---|---|
| 业务查询/条件 | `linq!(|e: E| ...)` → `Predicate<E>` | 编译期校验字段/类型，参数自动绑定 |
| 动态条件组合 | `Predicate::always/and_opt/or_opt` + 一次 `filter` | 禁止把请求字符串拼进 SQL |
| 固定报表 | `linq!` 的 `select (sum(...), count())` + `to_list_typed_*` | 编译期聚合项 |
| 动态报表（列/别名来自服务端白名单） | `select_items(&[SelectItem...])` | 标识符必须是常量/白名单；debug 构建校验 |
| 深翻页 | `KeysetRequest` + `CursorPage` | 不要自建游标 |
| 任意行 | `#[derive(FromRow)]` + `sql_query_rows` / `to_list_rows` | 不要求实体 |
| 复杂 SQL（框架表达不了） | raw SQL + `IRow`，或 raw CTE + 参数适配 | 用绑定参数，不做字符串拼接 |

## 4. 错误与并发语义

- DML：`ExecuteUpdateBuilder::compile_sql()` 单遍生成 SET → WHERE 参数；`execute()` 在
  provider 上执行，返回 affected rows（CAS 由调用方检查 0/1）。
- 事务：`use_transaction` 只 commit 一次；查询构建器不隐式开事务。
- provider 错误由边界 crate（`rust-webx-ef`）经 `map_ef_error` 统一映射，查询层不感知 HTTP。

## 5. 边界与拒绝策略（2.8）

- **raw 片段**（raw CTE / 集合运算操作数）：编译前校验占位符与参数个数（`?` 数量或
  `$N` 最大编号必须等于参数个数；`?` 与 `$N` 混用直接拒绝）。`?` 片段在 PostgreSQL
  编译时替换为 `$N`，`$N` 片段按已发射参数重编号。片段内字符串字面量中的 `$N`/`?`
  不在词法层区分，属专家 API 的已知限制。
- **typed CTE 谓词**：与 WHERE 一样解析相关子查询（`with ... as |b: T| ...` 中的
  `EXISTS`/`IN` 不会被编译成空表名）。
- **集合运算**：ORDER BY / LIMIT / OFFSET 在编译时位于所有集合运算之后（作用于合并
  结果）；COUNT/SUM/AVG/MIN/MAX 等单值终端与集合运算/分页/排序/窗口组合会被拒绝。
- **DML**：批量 `UPDATE`/`DELETE` 只支持 `FROM` + `WHERE`；JOIN/CTE/GROUP BY/HAVING/
  ORDER BY/分页/集合运算/DISTINCT/投影/窗口一律 typed error，避免子句被静默丢弃。
- **终端形状切换**：`count()`/`any()`/聚合终端会显式设置 `SelectList` 变体；分页与
  `COUNT` 的组合被拒绝（不会悄悄把 LIMIT 放在聚合行上），窗口投影与单值终端/EXISTS
  的组合同样被拒绝。
- **标识符**：表名/CTE 名/别名在 2.8 与既有 FROM 行为保持一致（按代码常量原样输出，
  由调用方保证是标识符）；相关子查询内部使用 `quote_identifier` 引用表/列。运行时
  标识符入口在 debug 构建有 `debug_assert_identifier`。3.0 计划统一引用策略。
- DML/导航/懒加载中仍有"编译 SQL + 另一遍取值"的调用点，但它们与主查询共用同一个
  表达式编译器，且 DML 侧加了参数数目断言；后续版本会收敛为单遍 helper（不影响当前
  正确性）。

## 6. 宏支持 API 的边界

`linq!` 需要把子句编译成方法链，因此 `QueryBuilder`/`SelectQueryBuilder`/`ExecuteUpdateBuilder`
上有一组 `#[doc(hidden)]` 的 `*_internal` 方法（`include_internal`、`join_internal`、
`with_cte_*`、`union_internal`、`window_internal` 等）。它们是**宏 codegen ABI**：不承诺
semver、不出现在 rustdoc 公共契约中，应用代码不应直接调用；2.8 已把其中重复的公开能力
（`group_by`/`having`/`order_by`/`select_columns`/聚合终端）改为公开方法，宏直接生成公开名。

## 7. 刻意保留的边界（2.8）

- 编译期无法表达任意 SQL：聚合窗口、复杂相关子查询、方言特有语法走 raw SQL + `IRow`。
- 运行时标识符 API（`SelectItem::column`、`order_by`、`ColumnExpr::column`、join keys）
  在 debug 构建做标识符断言，但 release 仍需调用方保证"常量/白名单"。
- 三方言执行验证在 CI 的 real-database job（PostgreSQL/MySQL service containers）完成；
  本地默认只跑 SQLite。
- 3.0 计划：`SelectList`/表达式 AST 的 `#[non_exhaustive]` 收口、进一步收敛公开配置结构体、
  raw 片段占位符的完整 AST 化。
