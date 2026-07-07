# 架构完善:query.rs / linq.rs 模块化拆分(继续推进)

## 概述

继续执行已批准的 [query-linq-modularization.md](file:///e:/GitCode/RF/rust-ef/.trae/documents/query-linq-modularization.md) 计划,将两个超大单文件拆分为子目录模块结构。本文档基于对当前代码状态的实测,细化具体执行步骤。

## 当前状态分析(Phase 1 探索结论)

### 文件规模实测
- [crates/core/src/query.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs) — **2771 行**(单文件,职责混杂:AST/状态/构建器/SQL 编译/辅助函数)
- [crates/macros/src/linq.rs](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs) — **2462 行**(单文件,职责混杂:AST/解析/代码生成/表达式编译/上下文)

### 代码当前状态:编译失败(关键阻塞)

`cargo check -p rust-ef-macros` 报错:
```
error[E0027]: pattern does not mention fields `recursive`, `link`
  --> crates\macros\src\linq.rs:1439:13
   |
1439 |             LinqClause::With {
1440 |                 name,
1441 |                 entity,
1442 |                 param,
1443 |                 body,
1444 |             } => {
```

**原因**:[linq.rs#L175-L183](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L175-L183) 的 `LinqClause::With` 变体已添加 `recursive: bool` 和 `link: Option<(Expr, Expr)>` 字段(为 Part 2 递归 CTE 准备);解析器 [parse_with_rest](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L953) 也已识别 `recursive`/`link` 关键字;但 [expand_clauses](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L1439) 的 `With` 分支未更新解构。

**对策**:在拆分 `linq/expand.rs` 时,完整实现 `With` 变体的递归 CTE 代码生成(同时解决编译错误和 Part 2 的 macro 侧剩余工作)。

### query.rs 模块边界实测

按代码实际段落,query.rs 顶部声明分布:

| 行号范围 | 内容 | 目标子模块 |
|----------|------|------------|
| L1-16 | imports + 模块文档 | 分散到各子模块 |
| L18-555 | AST 类型(FilterCondition, BoolExpr, SubquerySpec, InSubquerySpec, OrderBy, OrderDirection, CompiledFilter, IncludePath, JoinSpec, GroupBy, HavingCondition, AggKind, CompareOp, HavingExpr 及 impl) | `ast.rs` |
| L556-719 | 窗口函数(WindowFuncKind, WindowSpec 及 impl) | `window.rs` |
| L720-771 | CTE + 集合运算(CteSpec, SetOperator, SetOpSpec) | `cte.rs` |
| L773-1090 | QueryState 及 impl | `state.rs` |
| L1091-1170 | PortablePlaceholderGenerator + convert_aggregate_cell | `compile.rs`(SQL 生成辅助) |
| L1172-2437 | QueryBuilder\<T\> 及 impl(最大块,~1265 行) | `builder.rs` |
| L2439-2510 | ExecuteUpdateBuilder\<T\> | `execute_update.rs` |
| L2519-2667 | SelectQueryBuilder\<T\> | `select.rs` |
| L2669-2735 | LinqSource trait + ParseFromDb trait 及 impl + parse_column | `source.rs` |
| L2736-2962 | filters_to_and_expr, compile_bool_expr, compile_subquery, compile_in_subquery, collect_bool_expr_values, resolve_subqueries, has_subqueries, resolve_subquery_spec, build_where_clauses | `compile.rs` |
| L2963-2973 | like_contains, like_starts_with, like_ends_with | `helpers.rs` |
| L2982-末尾 | IQueryable trait | `source.rs` |

### linq.rs 模块边界实测

| 行号范围 | 内容 | 目标子模块 |
|----------|------|------------|
| L1-46 | imports + 模块文档 | 分散到各子模块 |
| L47-216 | AST(LinqInput, QueryInput, ValueInput, LinqClause, HavingExprAst, LinqOrder, ValueKind, JoinKind) | `ast.rs` |
| L218-1240 | 解析(Parse trait impl, parse_query, parse_value_*, parse_*_rest, collect_until_semi, parse_expr_until_fat_arrow_or_semi, is_source_expr, source_entity_type, expr_as_entity_type, parse_typed_closure, parse_closure_with_inference, parse_untyped_closure, parse_where_rest, parse_optional_order, parse_order_expr, parse_optional_clauses, expr_to_having_ast, parse_having_compare_from_binary, parse_agg_call, bin_op_to_symbol) | `parse.rs` |
| L1244-1620 | expand_linq, expand_query, is_true_expr, expand_clauses, expand_join, extract_field_array, extract_field_name_only, expand_value, expand_field_array | `expand.rs` |
| L1650-2395 | 表达式编译(compile_bool_expr, compile_bool_comparison, compile_bool_method, compile_expr, compile_not, compile_bool_member, compile_comparison, compile_negated_comparison, compile_contains, SubqueryKind, extract_subquery_closure, compile_subquery_parts, compile_subquery_method, compile_subquery_bool, compile_not_subquery, compile_in_subquery_parts, compile_in_subquery_method, compile_in_subquery_bool, compile_method, compile_order, compile_having_expr) | `compile.rs` |
| L1861-1927 | LinqCtx, FieldKind, FieldRef, extract_field | `context.rs` |

### 外部引用实测(API 兼容性约束)

`use rust_ef::query::*` / `use crate::query::*` 引用的所有类型(需在 `mod.rs` 通过 `pub use` 重新导出):
- 公开 API:`BoolExpr, FilterCondition, OrderBy, OrderDirection, QueryState, AggKind, CompareOp, HavingExpr, IQueryable, QueryBuilder, CompiledFilter, IncludePath, CteSpec, LinqSource, ParseFromDb, SetOperator, SetOpSpec, WindowFuncKind, WindowSpec`
- crate 内部:`compile_bool_expr, collect_bool_expr_values`(`pub(crate)` 可见性,被 `change_executor.rs`/`lazy.rs`/`navigation_loader.rs` 引用)

外部对 `linq` 模块仅引用 `rust_ef_macros::linq`(宏导出),无直接 `use ...::linq::*` 路径引用,因此 `linq/mod.rs` 只需 `pub use expand::expand_linq;`。

### lib.rs 现状

- [crates/core/src/lib.rs#L40](file:///e:/GitCode/RF/rust-ef/crates/core/src/lib.rs#L40):`pub mod query;` — 仍指向 `query/mod.rs`(无需修改)
- [crates/macros/src/lib.rs#L6](file:///e:/GitCode/RF/rust-ef/crates/macros/src/lib.rs#L6):`mod linq;` — 仍指向 `linq/mod.rs`(无需修改)
- [crates/core/src/lib.rs#L77-L80](file:///e:/GitCode/RF/rust-ef/crates/core/src/lib.rs#L77-L80):prelude 已含 `SetOperator, SetOpSpec`

## 提议变更

### Step 1:创建 `crates/core/src/query/` 子目录(11 个子模块)

**操作**:
1. 创建 `crates/core/src/query/` 目录
2. 创建 11 个子文件,按上文"query.rs 模块边界实测"切分
3. 创建 `mod.rs`,声明子模块并通过 `pub use` / `pub(crate) use` 重导出
4. 删除原 `crates/core/src/query.rs`

**子文件职责**:

| 文件 | 主要内容 |
|------|---------|
| `ast.rs` | L18-555 的所有 AST 类型及其 impl:`FilterCondition`、`BoolExpr`、`SubquerySpec`、`InSubquerySpec`、`OrderBy`、`OrderDirection`、`CompiledFilter`、`IncludePath`、`JoinSpec`、`GroupBy`、`HavingCondition`、`AggKind`、`CompareOp`、`HavingExpr` |
| `window.rs` | L556-719:`WindowFuncKind`、`WindowSpec` 及其 impl |
| `cte.rs` | L720-771:`CteSpec`(含 `is_recursive`/`recursive_link` 字段)、`SetOperator`、`SetOpSpec` |
| `state.rs` | L773-1090:`QueryState` 及其 impl |
| `compile.rs` | L1091-1170 (`PortablePlaceholderGenerator` + `convert_aggregate_cell`) + L2736-2962 (`filters_to_and_expr`、`compile_bool_expr`、`compile_subquery`、`compile_in_subquery`、`collect_bool_expr_values`、`resolve_subqueries`、`has_subqueries`、`resolve_subquery_spec`、`build_where_clauses`、`build_where_clause_with_offset`) |
| `builder.rs` | L1172-2437:`QueryBuilder<T>` 及其 impl |
| `execute_update.rs` | L2439-2510:`ExecuteUpdateBuilder<T>` |
| `select.rs` | L2519-2667:`SelectQueryBuilder<T>` |
| `source.rs` | L2669-2735 + L2982-末尾:`LinqSource` trait、`ParseFromDb` trait 及 impl、`parse_column`、`IQueryable` trait |
| `helpers.rs` | L2963-2973:`like_contains`、`like_starts_with`、`like_ends_with` |
| `mod.rs` | 模块声明 + 重导出(见下) |

**`mod.rs` 重导出策略**:

```rust
//! Query builder & LINQ-style chainable query API.
mod ast;
mod window;
mod cte;
mod state;
mod compile;
mod builder;
mod execute_update;
mod select;
mod source;
mod helpers;

pub use ast::*;
pub use window::*;
pub use cte::*;
pub use state::QueryState;
pub use builder::QueryBuilder;
pub use execute_update::ExecuteUpdateBuilder;
pub use select::SelectQueryBuilder;
pub use source::*;
pub use helpers::*;

// crate-internal helpers (used by change_executor / lazy / navigation_loader)
pub(crate) use compile::{
    collect_bool_expr_values, compile_bool_expr,
};
```

**跨子模块依赖**(子模块内部用 `use super::*` 引用):
- `builder.rs` → 需要 `use super::ast::{BoolExpr, JoinSpec, IncludePath, OrderBy, OrderDirection, GroupBy, HavingCondition, HavingExpr, CompiledFilter}`、`use super::state::QueryState`、`use super::cte::{CteSpec, SetOperator, SetOpSpec}`、`use super::compile::{compile_bool_expr, PortablePlaceholderGenerator, convert_aggregate_cell, filters_to_and_expr, collect_bool_expr_values, resolve_subqueries, has_subqueries, build_where_clauses}`、`use super::source::{LinqSource, ParseFromDb, IQueryable}`
- `state.rs` → 需要 `use super::ast::{BoolExpr, FilterCondition, OrderBy, IncludePath, JoinSpec, GroupBy, HavingCondition, HavingExpr}`、`use super::cte::{CteSpec, SetOpSpec}`、`use super::window::WindowSpec`
- `compile.rs` → 需要 `use super::ast::{BoolExpr, FilterCondition, SubquerySpec, InSubquerySpec}`、`use super::state::QueryState`
- `execute_update.rs` / `select.rs` → 需要 `use super::builder::QueryBuilder`、`use super::ast::*`
- `source.rs` → 需要 `use super::ast::*`、`use super::builder::QueryBuilder`

**imports 处理**:每个子文件按需添加以下 imports(从原 `query.rs` L8-16 拆分):
```rust
use crate::entity::{
    IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, ILazyInit, INavigationSetter,
};
use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, DbValueConvertError, IDatabaseProvider};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
```
(每个子文件只保留实际使用的 import,通过 `cargo check` 验证未使用警告)

### Step 2:创建 `crates/macros/src/linq/` 子目录(6 个子模块)

**操作**:
1. 创建 `crates/macros/src/linq/` 目录
2. 创建 6 个子文件,按上文"linq.rs 模块边界实测"切分
3. 创建 `mod.rs`,声明子模块并通过 `pub use expand::expand_linq;` 重导出
4. 删除原 `crates/macros/src/linq.rs`

**子文件职责**:

| 文件 | 主要内容 |
|------|---------|
| `ast.rs` | L47-216:`LinqInput`、`QueryInput`、`LinqOrder`、`ValueInput`、`LinqClause`(含 `With` 的 `recursive`/`link` 字段)、`HavingExprAst`、`ValueKind`、`JoinKind` |
| `parse.rs` | L218-1240:所有 parse_* 函数 + `Parse` trait impl + 辅助函数(is_source_expr, source_entity_type, expr_as_entity_type 等) |
| `expand.rs` | L1244-1620:`expand_linq`、`expand_query`、`is_true_expr`、`expand_clauses`、`expand_join`、`extract_field_array`、`extract_field_name_only`、`expand_value`、`expand_field_array` |
| `compile.rs` | L1650-2395:所有 compile_* 函数 + `SubqueryKind` 及其 impl + `extract_subquery_closure` |
| `context.rs` | L1861-1927:`LinqCtx`、`FieldKind`、`FieldRef`、`extract_field` |
| `mod.rs` | 模块声明 + `pub use expand::expand_linq;` |

**`mod.rs` 内容**:

```rust
//! `linq!()` — compile-time LINQ-to-SQL.
mod ast;
mod parse;
mod expand;
mod compile;
mod context;

pub use expand::expand_linq;
```

**跨子模块依赖**:
- `parse.rs` → 需要 `use super::ast::*`
- `expand.rs` → 需要 `use super::ast::*`、`use super::compile::compile_bool_expr`、`use super::context::{LinqCtx, extract_field, extract_field_array, extract_field_name_only}`、`use super::parse::JoinKind`(若 expand_join 需要)
- `compile.rs` → 需要 `use super::ast::{HavingExprAst, LinqClause}`(若有交叉)、`use super::context::{LinqCtx, FieldKind, FieldRef, extract_field}`
- `context.rs` → 自包含(可能需要 `use super::ast::LinqClause`)

**imports 处理**:每个子文件按需添加(从原 `linq.rs` L35-41 拆分):
```rust
use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::{
    parse::Parse, parse_macro_input, BinOp, Expr, ExprClosure, ExprField, ExprLit, ExprMethodCall,
    ExprPath, ExprUnary, Ident, Lit, Member, Pat, Token, Type, UnOp,
};
```

### Step 3:在 `linq/expand.rs` 中完整实现 `With` 变体递归 CTE 代码生成(修复编译错误)

**目标**:解决 [linq.rs#L1439](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L1439) 的 `E0027` 编译错误,并完成 Part 2 macro 侧剩余工作。

**修改内容**(`expand.rs` 中 `LinqClause::With` 分支):

```rust
LinqClause::With {
    name,
    entity,
    param,
    body,
    recursive,
    link,
} => {
    let cte_ctx = LinqCtx::single(entity, Some(param));
    let bool_expr_code = compile_bool_expr(&cte_ctx, body)?;
    let name_str = name.as_str();

    if recursive {
        // link must be present for recursive CTEs
        let (fk_expr, pk_expr) = link.expect("recursive CTE must have `link <fk> to <pk>`");
        let fk_col = extract_field_name_only(&fk_expr)?;
        let pk_col = extract_field_name_only(&pk_expr)?;
        chain = quote! {
            #chain .with_recursive_cte_typed(
                #name_str,
                <#entity>::TABLE,
                #fk_col,
                #pk_col,
                #bool_expr_code,
            )
        };
    } else {
        chain = quote! {
            #chain .with_cte_typed(
                #name_str,
                <#entity>::TABLE,
                #bool_expr_code,
            )
        };
    }
}
```

**依赖**:确认 `with_recursive_cte_typed` 方法签名已在 [query.rs#L1885](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1885) 实现(已确认存在);`extract_field_name_only` 函数已定义(在 L1601,会迁入 `expand.rs` 或 `context.rs`,按职责迁入 `context.rs`)。

**注意**:`extract_field_name_only` 当前位于 L1601,介于 `expand.rs` 区段(L1244-1620)和 `compile.rs` 区段(L1650-2395)边界附近。按职责(纯字段名提取,无 LinqCtx 依赖),归入 `context.rs` 更合适。

### Step 4:验证编译和测试

```bash
cargo check --workspace
cargo build --workspace
cargo test --workspace -- --exclude-threads 1
```

**期望结果**:
- `cargo check` 无错误无警告(除 dead_code 警告外)
- `cargo test` 全部通过(忽略环境相关的 PostgreSQL/MySQL 测试失败)
- 所有现有测试无 API 破坏(`use rust_ef::query::*` 路径不变)

### Step 5:更新 CHANGELOG

在 [CHANGELOG.md](file:///e:/GitCode/RF/rust-ef/CHANGELOG.md) 追加变更条目:

```markdown
### Architecture Refactor

- Split `crates/core/src/query.rs` (2771 lines) into `query/` subdirectory
  with 11 responsibility-focused child modules (`ast`, `window`, `cte`,
  `state`, `compile`, `builder`, `execute_update`, `select`, `source`,
  `helpers`, `mod`).
- Split `crates/macros/src/linq.rs` (2462 lines) into `linq/` subdirectory
  with 6 child modules (`ast`, `parse`, `expand`, `compile`, `context`,
  `mod`).
- Completed `with recursive ... link <fk> to <pk>` macro codegen (Part 2
  recursive CTE macro side).
- API compatibility preserved: all `use rust_ef::query::*` paths continue
  to work via `pub use` re-exports in `query/mod.rs`.
```

## 假设与决策

1. **API 兼容优先**:所有现有 `use rust_ef::query::{...}` / `use crate::query::{...}` 路径必须零改动;通过 `pub use` 重导出保证
2. **lib.rs 不动**:`pub mod query;` / `mod linq;` 声明不变
3. **prelude 不动**:现有 prelude 导出已正确(含 `SetOperator, SetOpSpec`)
4. **子模块间用相对路径**:`use super::ast::BoolExpr` 等,避免绝对路径
5. **不引入新抽象**:本次只做物理拆分,不重命名类型/方法,不调整 API 签名
6. **Part 2 macro 侧一并完成**:`With` 变体的递归代码生成在 `expand.rs` 中实现,作为拆分的一部分(否则拆分后无法编译)
7. **后续 Priority 3 Parts 暂不实施**:Part 4 (CASE WHEN) 和 Part 5 (UPSERT) 留待拆分完成后,在新的模块结构上单独推进
8. **`builder.rs` 单文件 ~1265 行可接受**:QueryBuilder 是单一类型的多组方法,跨文件拆分 impl 块会增加复杂度,不符合 Rust 习惯

## 实施顺序(任务清单)

- [ ] **Task 1**:创建 `crates/core/src/query/` 目录 + 11 个子文件,迁移内容(保持行号对应)
- [ ] **Task 2**:创建 `query/mod.rs` 含 `pub use` / `pub(crate) use` 重导出
- [ ] **Task 3**:删除原 `query.rs`,运行 `cargo check -p rust-ef` 验证编译
- [ ] **Task 4**:创建 `crates/macros/src/linq/` 目录 + 6 个子文件,迁移内容
- [ ] **Task 5**:在 `linq/expand.rs` 中实现 `With` 变体完整解构 + 递归 CTE 代码生成
- [ ] **Task 6**:创建 `linq/mod.rs` 含 `pub use expand::expand_linq;`
- [ ] **Task 7**:删除原 `linq.rs`,运行 `cargo check --workspace` 验证编译
- [ ] **Task 8**:`cargo test --workspace` 全量验证
- [ ] **Task 9**:更新 CHANGELOG

## 验证清单

1. `cargo check --workspace` — 编译通过,无 E0027 等错误
2. `cargo build --workspace` — 全工作区编译通过
3. `cargo test --workspace` — 所有测试通过(忽略环境相关的 PG/MySQL 失败)
4. API 兼容性:`Grep "use (rust_ef|crate)::query::"` 引用路径全部仍可解析
5. 文件结构:`Get-ChildItem crates/core/src/query/` 返回 11 个 `.rs` 文件;`crates/macros/src/linq/` 返回 6 个 `.rs` 文件

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| 子模块间循环依赖 | 严格分层:`ast` ← `cte`/`window` ← `state` ← `compile` ← `builder` ← `execute_update`/`select`/`source`;`helpers` 独立;`compile.rs` 不依赖 `builder.rs` |
| 可见性错误(pub vs pub(crate)) | `mod.rs` 中 `pub use` 重导出原始可见性;`pub(crate)` 项目用 `pub(crate) use` 显式重导出 |
| imports 未使用警告 | 每个子文件按 `cargo check` 提示精简 imports |
| `extract_field_name_only` 跨模块引用 | 归入 `context.rs`,被 `expand.rs` 和 `compile.rs` 共享 |
| QueryBuilder impl 块过大 | 暂不拆分;若未来需要,可考虑 trait 拆分(如 `QueryBuilderCteExt`),但不在本次范围 |
