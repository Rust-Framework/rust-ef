# 架构完善:query.rs 和 linq.rs 模块化拆分

## 背景

当前 `crates/core/src/query.rs` (2771行) 和 `crates/macros/src/linq.rs` (2462行) 严重超标,单文件堆积了查询 AST、状态管理、构建器、SQL 编译、宏解析、代码生成等多重职责。这违背了"职责清晰、模块化开发"的架构原则,不利于演进迭代和稳定性保障。

用户要求采用**子目录模式**进行拆分,并优先完成架构拆分后再继续 Priority 3 功能开发。

## 拆分方案

### 目标结构

**`crates/core/src/query/` 子目录(替代 `query.rs`):**

| 文件 | 职责 | 来源行号 | 约行数 |
|------|------|----------|--------|
| `mod.rs` | 模块声明 + pub re-exports(保持 API 兼容) | 新建 | ~60 |
| `ast.rs` | 查询表达式 AST 类型 | L18-555 | ~540 |
| `window.rs` | 窗口函数(WindowFuncKind, WindowSpec) | L556-700 | ~145 |
| `cte.rs` | CTE 和集合运算(CteSpec, SetOperator, SetOpSpec) | L700-760 | ~60 |
| `state.rs` | QueryState 累积器 | L767-1135 | ~370 |
| `builder.rs` | QueryBuilder<T>(核心构建器) | L1140-2430 | ~1290 |
| `execute_update.rs` | ExecuteUpdateBuilder<T> | L2437-2511 | ~75 |
| `select.rs` | SelectQueryBuilder<T> | L2517-2656 | ~140 |
| `source.rs` | LinqSource / ParseFromDb / IQueryable traits | L2669-2975 | ~310 |
| `compile.rs` | compile_bool_expr / filters_to_and_expr(SQL 编译) | L2749-2746(散布) | ~250 |
| `helpers.rs` | like_contains / like_starts_with / like_ends_with | L2958-2975 | ~20 |

**`crates/macros/src/linq/` 子目录(替代 `linq.rs`):**

| 文件 | 职责 | 来源行号 | 约行数 |
|------|------|----------|--------|
| `mod.rs` | expand_linq 入口 + 模块声明 | L1244-1254 | ~30 |
| `ast.rs` | AST 类型(LinqInput, QueryInput, ValueInput, LinqClause, HavingExprAst, LinqOrder, JoinKind) | L47-216 | ~170 |
| `parse.rs` | 解析函数(Parse trait, parse_query, parse_*_rest) | L218-1240 | ~1020 |
| `expand.rs` | 代码生成(expand_query, expand_clauses, expand_value, expand_join) | L1256-1620 | ~365 |
| `compile.rs` | 表达式编译(compile_bool_expr, compile_expr, compile_comparison, compile_subquery_*, compile_having_expr) | L1621-2250 | ~630 |
| `context.rs` | LinqCtx + 辅助函数(extract_field, is_true_expr 等) | L1857-2610 | ~280 |

### 拆分原则

1. **API 兼容**:每个 `mod.rs` 通过 `pub use` 重新导出所有 pub 类型/函数,确保 `use crate::query::QueryBuilder` 等现有引用零改动
2. **lib.rs 不变**:`pub mod query;` 仍然指向 `query/mod.rs`;`pub mod` 声明不改
3. **prelude 不变**:lib.rs prelude 的 re-export 路径不变
4. **测试不改**:所有测试通过 `use rust_ef::...` 或 prelude 引用,拆分后仍兼容
5. **跨模块依赖**:子模块间用 `use super::ast::BoolExpr` 等相对路径引用

### 关键依赖关系

**query/ 内部依赖:**
- `builder.rs` → 依赖 `ast.rs`(BoolExpr, JoinSpec 等)、`state.rs`(QueryState)、`cte.rs`(CteSpec)、`compile.rs`(compile_bool_expr)
- `state.rs` → 依赖 `ast.rs`(所有 AST 类型)、`cte.rs`(CteSpec, SetOpSpec)
- `compile.rs` → 依赖 `ast.rs`(BoolExpr, FilterCondition)
- `execute_update.rs` / `select.rs` → 依赖 `builder.rs`、`ast.rs`

**linq/ 内部依赖:**
- `expand.rs` → 依赖 `ast.rs`(LinqClause, QueryInput)、`compile.rs`(compile_bool_expr)、`context.rs`(LinqCtx, extract_field)
- `parse.rs` → 依赖 `ast.rs`(LinqClause, HavingExprAst)
- `compile.rs` → 依赖 `context.rs`(LinqCtx)、`ast.rs`(HavingExprAst)

## 实施步骤

### Step 1:创建 query/ 子目录结构

1. 创建 `crates/core/src/query/` 目录
2. 将 `query.rs` 内容按职责拆分到子文件:
   - `ast.rs`:FilterCondition, BoolExpr, SubquerySpec, InSubquerySpec, OrderBy, OrderDirection, CompiledFilter, IncludePath, JoinSpec, GroupBy, HavingCondition, AggKind, CompareOp, HavingExpr 及其 impl
   - `window.rs`:WindowFuncKind, WindowSpec
   - `cte.rs`:CteSpec, SetOperator, SetOpSpec
   - `state.rs`:QueryState 及其 impl
   - `builder.rs`:QueryBuilder<T> 及其 impl(最大块,~1290行)
   - `execute_update.rs`:ExecuteUpdateBuilder<T>
   - `select.rs`:SelectQueryBuilder<T>
   - `source.rs`:LinqSource, ParseFromDb, IQueryable traits
   - `compile.rs`:compile_bool_expr, filters_to_and_expr, collect_bool_expr_values
   - `helpers.rs`:like_contains, like_starts_with, like_ends_with
3. 创建 `mod.rs`:
   ```rust
   mod ast;
   mod window;
   mod cte;
   mod state;
   mod builder;
   mod execute_update;
   mod select;
   mod source;
   mod compile;
   mod helpers;

   pub use ast::*;
   pub use window::*;
   pub use cte::*;
   pub use state::QueryState;
   pub use builder::QueryBuilder;
   pub use execute_update::ExecuteUpdateBuilder;
   pub use select::SelectQueryBuilder;
   pub use source::*;
   pub use compile::*;
   pub use helpers::*;
   ```
4. 删除原 `query.rs`

### Step 2:创建 linq/ 子目录结构

1. 创建 `crates/macros/src/linq/` 目录
2. 将 `linq.rs` 内容按职责拆分:
   - `ast.rs`:LinqInput, QueryInput, ValueInput, LinqClause, HavingExprAst, LinqOrder, JoinKind, ValueKind
   - `parse.rs`:Parse trait impl, parse_query, parse_value_*, parse_*_rest, collect_until_semi, parse_expr_until_fat_arrow_or_semi
   - `expand.rs`:expand_query, expand_clauses, expand_value, expand_join, is_true_expr
   - `compile.rs`:compile_bool_expr, compile_bool_comparison, compile_bool_method, compile_expr, compile_not, compile_comparison, compile_negated_comparison, compile_contains, compile_subquery_*, compile_not_subquery, compile_having_expr
   - `context.rs`:LinqCtx, extract_field, extract_field_array, extract_field_name_only, extract_field_nav, extract_value
3. 创建 `mod.rs`:
   ```rust
   mod ast;
   mod parse;
   mod expand;
   mod compile;
   mod context;

   pub use expand::expand_linq;
   ```
4. 更新 `crates/macros/src/lib.rs`:`pub mod linq;` 仍指向 `linq/mod.rs`
5. 删除原 `linq.rs`

### Step 3:验证编译和测试

```bash
cargo build --workspace
cargo test --workspace
```

确保所有现有测试通过,API 无破坏性变更。

### Step 4:继续 Priority 3 剩余功能

在新的模块结构上继续实现:
- Part 2:递归 CTE(在 `query/cte.rs` 和 `linq/parse.rs` + `linq/expand.rs` 上)
- Part 4:CASE WHEN(在 `query/ast.rs` + `query/compile.rs` 和 `linq/compile.rs` 上)
- Part 5:UPSERT(在 `entity.rs` + `db_set.rs` + `provider.rs` + `change_executor.rs` 上)

## 验证

1. `cargo build --workspace` — 编译通过
2. `cargo test --workspace` — 所有测试通过(忽略环境相关的 PG/MySQL 失败)
3. API 兼容性:`grep -r "use rust_ef::query::" crates/` 引用路径不变
4. prelude 导出不变:`use rust_ef::prelude::*` 仍可用

## 注意事项

- `query/builder.rs` 仍有 ~1290 行,但它是 QueryBuilder 的单一职责实现,方法按功能分组(filter/join/order/group/aggregate/terminal/cte/set_op/window)。进一步拆分会导致 QueryBuilder 类型分散,不符合 Rust 的 impl 块规则。如需进一步拆分,可考虑 trait 拆分(如 `QueryBuilderJoinExt` trait),但这会增加复杂度,暂不推荐。
- `linq/parse.rs` 仍有 ~1020 行,但所有解析函数都是 LinqClause::parse 的辅助,职责单一。如需进一步拆分,可按子句类型分组(如 `parse/join.rs`、`parse/window.rs`),但暂不推荐过度拆分。
