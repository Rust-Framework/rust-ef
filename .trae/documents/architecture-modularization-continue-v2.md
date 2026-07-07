# 架构完善:query.rs / linq.rs 模块化拆分(继续推进 v2)

## 概述

本文档接续 [architecture-modularization-continue.md](file:///e:/GitCode/RF/rust-ef/.trae/documents/architecture-modularization-continue.md)(已批准),记录在实际执行阶段已部分完成后剩余的具体工作。用户指令:"架构设计要求职责清晰、模块化开发,避免一个代码文件堆积大量逻辑,不利于架构演进迭代和维护,无法确保稳定性,请继续推进架构完善"。

## 当前状态实测(Phase 1 探索)

### 已完成

**`crates/core/src/query/` 子目录(9/11 文件已创建):**
- [ast.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/ast.rs) — AST 类型(L18-555)
- [window.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/window.rs) — 窗口函数(L556-719)
- [cte.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/cte.rs) — CTE + 集合运算(L720-771)
- [state.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/state.rs) — QueryState(L773-1090)
- [compile.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/compile.rs) — SQL 编译辅助(L1091-1170 + L2736-2962)
- [source.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/source.rs) — LinqSource/ParseFromDb/IQueryable(L2669-2735 + 末尾)
- [execute_update.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/execute_update.rs) — ExecuteUpdateBuilder(L2439-2510)
- [select.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/select.rs) — SelectQueryBuilder(L2519-2667)
- [helpers.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query/helpers.rs) — like_* 辅助(L2963-2973)

**已验证:** 9 个文件的 imports、跨模块引用(`use super::ast::*`、`use super::compile::*`)、可见性(`pub(crate)`)均正确,无未使用 import 警告。

### 待完成

1. **创建 `crates/core/src/query/builder.rs`** — QueryBuilder\<T\> 及 impl(原 [query.rs#L1163-L2431](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1163-L2431),约 1268 行)
2. **创建 `crates/core/src/query/mod.rs`** — 模块声明 + `pub use` / `pub(crate) use` 重导出
3. **删除原 `crates/core/src/query.rs`**
4. **运行 `cargo check -p rust-ef`** 验证 query/ 拆分编译通过
5. **创建 `crates/macros/src/linq/` 子目录(6 个子模块)**,迁移原 [linq.rs](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs)(2462 行)内容
6. **在 `linq/expand.rs` 中修复 E0027**:`LinqClause::With` 分支补全 `recursive`、`link` 字段解构,并实现递归 CTE 代码生成
7. **删除原 `crates/macros/src/linq.rs`**
8. **运行 `cargo check --workspace` + `cargo test --workspace`** 全量验证
9. **更新 [CHANGELOG.md](file:///e:/GitCode/RF/rust-ef/CHANGELOG.md)**

### 关键阻塞:编译错误 E0027

[linq.rs#L1439-L1444](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L1439) 的 `LinqClause::With` 解构缺少 `recursive`、`link` 字段(变体定义在 [L175-L183](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L175-L183) 已含 6 字段)。这是 Part 2 递归 CTE 准备工作的遗留,将在拆分 `linq/expand.rs` 时一并修复(同时完成 Part 2 macro 侧)。

## 提议变更

### Step 1:完成 `query/` 子目录(创建 builder.rs + mod.rs)

#### 1.1 创建 `crates/core/src/query/builder.rs`

**内容来源:** 原 [query.rs#L1163-L2431](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1163-L2431)

**结构:**
- `QueryBuilder<T: IEntityType>` 结构体定义(5 个字段:`state`、`provider`、`filter_map`、`lazy_loading_enabled`、`_phantom`)
- `impl<T: IEntityType> QueryBuilder<T>` 整块实现,包含的方法组:
  - 构造:`new`、`with_provider`、`with_filter_map`、`with_lazy_loading`
  - 状态访问:`state`
  - 过滤(filter / filter_column / filter_not / filter_in / filter_not_in / filter_is_null / filter_is_not_null / filter_between / filter_like / filter_not_like / or_where / apply_query_filter / where_exists_internal / where_in_subquery_internal)
  - 排序(order_by_column / order_by_desc_column)
  - 分页(skip / take)
  - Include(include_internal / then_include_internal)
  - JOIN(inner_join_internal / left_join_internal / right_join_internal / full_join_internal / cross_join_internal)
  - GROUP BY / HAVING(group_by_internal / having_internal / having_expr_internal)
  - 窗口函数(window_internal)
  - CTE(with_cte_internal / with_cte_typed / with_recursive_cte_typed / from_cte)
  - 集合运算(union_internal / union_all_internal / intersect_internal / except_internal)
  - 聚合终端(sum_internal / avg_internal / min_internal / max_internal)
  - 投影(select_internal)
  - SQL 生成(to_sql / compile_sql / compile_state_sql)
  - 终端方法(to_list / to_list_with_includes / first / first_or_default / count / any / last / last_or_default / single / single_or_default / long_count / all / contains / to_dictionary / find / find_by_key / exists_by_id / exists_by_key)
  - 批量操作(execute_update / execute_delete)
  - distinct

**文件头 imports(精简到实际使用):**

```rust
//! `QueryBuilder<T>` — chainable query builder for entity type `T`.
//!
//! Corresponds to EFCore's `IQueryable<T>`. Accumulates filter conditions,
//! orderings, pagination, includes, projections, and CTE/window/set-op
//! state via a fluent interface. Terminal methods (`to_list`, `first`,
//! `count`, etc.) compile the state into SQL and execute it against the
//! attached provider.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::entity::{
    IEntitySnapshot, IEntityType, IFromRow, IGetKeyValues, ILazyInit, INavigationSetter,
};
use crate::error::EFResult;
use crate::provider::{DbValue, DbValueConvertError, IDatabaseProvider};

use super::ast::{
    AggKind, BoolExpr, CompareOp, CompiledFilter, FilterCondition, HavingExpr, IncludePath,
    InSubquerySpec, JoinSpec, OrderBy, OrderDirection, SubquerySpec,
};
use super::compile::{
    build_where_clauses, collect_bool_expr_values, compile_bool_expr, convert_aggregate_cell,
    filters_to_and_expr, has_subqueries, resolve_subqueries, PortablePlaceholderGenerator,
};
use super::cte::{CteSpec, SetOpSpec, SetOperator};
use super::execute_update::ExecuteUpdateBuilder;
use super::select::SelectQueryBuilder;
use super::state::QueryState;
use super::window::{WindowFuncKind, WindowSpec};
```

**可见性调整:**
- 结构体字段 `state`、`provider`、`filter_map`、`lazy_loading_enabled`、`_phantom` 保持 `private`(原文件即如此,被 `execute_update.rs` / `select.rs` 通过 `pub(crate)` 字段重写规避 — 见下)
- 由于 `ExecuteUpdateBuilder::new` 内部需要从 `QueryBuilder` 构造(原 query.rs L2392-2397 直接访问 `self.state`、`self.provider`),`execute_update.rs` 与 `select.rs` 已将自身字段改为 `pub(crate)`,由 `QueryBuilder::execute_update` / `QueryBuilder::select_internal` 直接构造它们 — 这条链路已通过 9 个已创建文件验证
- `QueryBuilder` 自身字段保持 private,仅通过方法暴露

#### 1.2 创建 `crates/core/src/query/mod.rs`

**内容(完整):**

```rust
//! Query builder & LINQ-style chainable query API.
//!
//! Accumulates filter conditions, orderings, pagination, includes, and
//! projection metadata through a fluent interface. Terminal methods
//! (`to_list`, `first`, `count`, etc.) produce real SQL that can be
//! executed against a database provider.

mod ast;
mod builder;
mod compile;
mod cte;
mod execute_update;
mod helpers;
mod select;
mod source;
mod state;
mod window;

pub use ast::*;
pub use builder::QueryBuilder;
pub use cte::*;
pub use execute_update::ExecuteUpdateBuilder;
pub use helpers::*;
pub use select::SelectQueryBuilder;
pub use source::*;
pub use state::QueryState;
pub use window::*;

// crate-internal helpers (used by change_executor / lazy / navigation_loader)
pub(crate) use compile::{collect_bool_expr_values, compile_bool_expr};
```

**说明:**
- 11 个子模块声明(含 `mod.rs` 自身)
- `pub use ast::*` 等通过通配符重导出,确保 `use rust_ef::query::{BoolExpr, QueryBuilder, ...}` 路径不变(API 兼容)
- `pub(crate) use compile::{...}` 显式重导出 crate 内部使用的 `compile_bool_expr` / `collect_bool_expr_values`(被 [change_executor.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs)、[lazy.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/lazy.rs)、[navigation_loader.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/navigation_loader.rs) 引用)

#### 1.3 删除原 `crates/core/src/query.rs`

使用 `DeleteFile` 工具。Rust 模块解析会自动从 `query/mod.rs` 加载(因为 `lib.rs` 的 `pub mod query;` 同时支持文件和目录两种形式)。

#### 1.4 验证

```powershell
cargo check -p rust-ef
```

**期望:** 编译通过,无错误无警告(除可能的 dead_code 警告外)。如果出现 imports 缺失/未使用,逐个文件用 `Edit` 修正。

### Step 2:创建 `linq/` 子目录(6 个子模块)

#### 2.1 模块边界(基于实测行号)

| 子文件 | 行号范围 | 内容 |
|--------|---------|------|
| `ast.rs` | L47-216 | `LinqInput`、`QueryInput`、`LinqOrder`、`ValueInput`、`LinqClause`(含 `With` 的 `recursive`/`link` 字段)、`HavingExprAst` |
| `parse.rs` | L218-1240 | `impl Parse for LinqInput`、`ValueKind`、`JoinKind`、所有 `parse_*` 函数、`is_source_expr`、`source_entity_type`、`expr_as_entity_type`、`bin_op_to_symbol`、`parse_agg_call`、`expr_to_having_ast` |
| `expand.rs` | L1244-1648 | `expand_linq`、`expand_query`、`is_true_expr`、`expand_clauses`、`expand_join`、`extract_field_array`、`extract_field_name_only`、`expand_value`、`expand_field_array` |
| `compile.rs` | L1650-1843 + L1909-2395 + L2396-2641 | `compile_bool_expr`、`compile_bool_comparison`、`compile_bool_method`、`compile_expr`、`compile_not`、`compile_bool_member`、`compile_comparison`、`compile_negated_comparison`、`compile_contains`、`SubqueryKind` + impl、`extract_subquery_closure`、`compile_subquery_*`、`compile_in_subquery_*`、`compile_not_subquery`、`compile_method`、`compile_order`、`compile_having_expr`、`agg_kind_ident`、`op_to_ident` |
| `context.rs` | L1861-1927 + L2410-2543 | `LinqCtx`、`FieldKind`、`FieldRef`、`extract_field`、`extract_field_nav`、`extract_field_ref`、`type_path_matches`、`field_const`、`extract_value` |
| `mod.rs` | 新建 | 模块声明 + `pub use expand::expand_linq;` |

**说明:** `compile.rs` 包含 L1909 起的 `compile_expr` 及之后所有 `compile_*` 函数,以及 L2396 的 `compile_order`、L2543 的 `compile_having_expr`、L2611 的 `agg_kind_ident`、L2625 的 `op_to_ident`。`context.rs` 包含 L1861 的 `LinqCtx`/`FieldKind`/`FieldRef` 三类定义,以及 L2410 起的字段提取辅助函数(`extract_field`、`extract_field_nav`、`extract_field_ref`、`type_path_matches`、`field_const`、`extract_value`)。

**`extract_field_name_only`(L1601)保留在 `expand.rs`** — 它不依赖 `LinqCtx`,且仅被 `expand.rs` 内部使用(L1362 和待实现的 `With` 变体递归代码生成)。原计划提到归入 `context.rs`,但实测后认为保留在 `expand.rs` 更贴近使用点,减少跨模块引用。

#### 2.2 跨子模块依赖

- `parse.rs` → `use super::ast::*;`
- `expand.rs` → `use super::ast::*;`、`use super::compile::compile_bool_expr;`、`use super::context::{LinqCtx, extract_field, extract_field_array, extract_field_name_only, field_const, extract_value};`(注意:`extract_field_array` 实际定义在 `expand.rs` 内,不跨模块)
  - **修正:** `extract_field_array`(L1591)和 `extract_field_name_only`(L1601)实际位于 `expand.rs` 区段内,作为本模块私有函数;`expand.rs` 仅需 `use super::compile::compile_bool_expr;` 和 `use super::context::{LinqCtx, extract_field, field_const, extract_value};`
- `compile.rs` → `use super::ast::{HavingExprAst, LinqClause};`(若有交叉)、`use super::context::{LinqCtx, FieldKind, FieldRef, extract_field, extract_field_ref, field_const, extract_value, type_path_matches};`
- `context.rs` → `use super::ast::LinqClause;`(若需要)、`use proc_macro2::TokenStream2;`、`use syn::{...};`

#### 2.3 修复 E0027(在 `linq/expand.rs` 中)

将原 `LinqClause::With` 分支:

```rust
LinqClause::With {
    name,
    entity,
    param,
    body,
} => {
    let cte_ctx = LinqCtx::single(entity, Some(param));
    let bool_expr_code = compile_bool_expr(&cte_ctx, body)?;
    let name_str = name.as_str();
    chain = quote! {
        #chain .with_cte_typed(
            #name_str,
            <#entity>::TABLE,
            #bool_expr_code,
        )
    };
}
```

替换为:

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
        let (fk_expr, pk_expr) = link.expect(
            "recursive CTE must have `link <fk> to <pk>`"
        );
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

**依赖验证:**
- `QueryBuilder::with_recursive_cte_typed` 已在原 [query.rs#L1885-L1906](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1885) 实现(将迁入 `query/builder.rs`)
- `extract_field_name_only` 在 `expand.rs` 本模块内(原 L1601)
- `LinqCtx::single` 在 `context.rs`(原 L1861 区段)

#### 2.4 创建 `linq/mod.rs`

```rust
//! `linq!()` — compile-time LINQ-to-SQL.
//!
//! The single DSL entry point for all database operations. See the crate
//! root docs for usage examples.

mod ast;
mod compile;
mod context;
mod expand;
mod parse;

pub use expand::expand_linq;
```

#### 2.5 删除原 `crates/macros/src/linq.rs`

使用 `DeleteFile` 工具。`crates/macros/src/lib.rs` 的 `mod linq;`(L6)会自动从 `linq/mod.rs` 加载。

### Step 3:全量验证

```powershell
cargo check --workspace
cargo build --workspace
cargo test --workspace
```

**期望结果:**
- `cargo check` 无 E0027 错误,无 warnings(除 dead_code)
- `cargo test` 全部通过(忽略 PostgreSQL/MySQL 环境相关失败)
- API 兼容性:`use rust_ef::query::*` / `use rust_ef_macros::linq` 路径全部仍可解析

### Step 4:更新 CHANGELOG

在 [CHANGELOG.md](file:///e:/GitCode/RF/rust-ef/CHANGELOG.md) 追加:

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

1. **API 兼容优先**:所有 `use rust_ef::query::{...}` / `use crate::query::{...}` / `use rust_ef_macros::linq` 路径必须零改动,通过 `pub use` / `pub(crate) use` 重导出保证
2. **lib.rs 不动**:`pub mod query;` / `mod linq;` 声明不变
3. **prelude 不动**:[lib.rs#L77-L80](file:///e:/GitCode/RF/rust-ef/crates/core/src/lib.rs#L77) 现有 prelude 已正确(含 `SetOperator, SetOpSpec`)
4. **子模块间用相对路径**:`use super::ast::BoolExpr` 等,避免绝对路径
5. **不引入新抽象**:本次只做物理拆分,不重命名类型/方法,不调整 API 签名
6. **Part 2 macro 侧一并完成**:`With` 变体的递归代码生成在 `expand.rs` 中实现,作为拆分的一部分(否则拆分后无法编译)
7. **`extract_field_name_only` 保留在 `expand.rs`**:实测后认为它仅被 `expand.rs` 使用,无需归入 `context.rs`(修正原计划的"注意"项)
8. **`builder.rs` 单文件 ~1268 行可接受**:QueryBuilder 是单一类型的多组方法,跨文件拆分 impl 块会增加复杂度,不符合 Rust 习惯
9. **后续 Priority 3 Parts 暂不实施**:Part 4 (CASE WHEN) 和 Part 5 (UPSERT) 留待拆分完成后,在新的模块结构上单独推进

## 实施顺序(任务清单)

- [ ] **Task 1**:创建 `crates/core/src/query/builder.rs`(原 L1163-L2431 内容 + 精简 imports)
- [ ] **Task 2**:创建 `crates/core/src/query/mod.rs`(11 子模块声明 + 重导出)
- [ ] **Task 3**:删除原 `crates/core/src/query.rs`
- [ ] **Task 4**:运行 `cargo check -p rust-ef` 验证 query/ 拆分编译通过;若有 imports 问题用 `Edit` 逐个修正
- [ ] **Task 5**:创建 `crates/macros/src/linq/` 目录 + 5 个子文件(ast/parse/expand/compile/context)
- [ ] **Task 6**:在 `linq/expand.rs` 中实现 `With` 变体完整解构 + 递归 CTE 代码生成(修复 E0027)
- [ ] **Task 7**:创建 `linq/mod.rs`(`pub use expand::expand_linq;`)
- [ ] **Task 8**:删除原 `crates/macros/src/linq.rs`
- [ ] **Task 9**:运行 `cargo check --workspace` 验证编译通过;若有 imports 问题用 `Edit` 逐个修正
- [ ] **Task 10**:运行 `cargo test --workspace` 全量验证
- [ ] **Task 11**:更新 [CHANGELOG.md](file:///e:/GitCode/RF/rust-ef/CHANGELOG.md)

## 验证清单

1. `cargo check --workspace` — 编译通过,无 E0027 等错误
2. `cargo build --workspace` — 全工作区编译通过
3. `cargo test --workspace` — 所有测试通过(忽略环境相关的 PG/MySQL 失败)
4. API 兼容性:`Grep "use (rust_ef|crate)::query::"` 引用路径全部仍可解析
5. 文件结构:`Get-ChildItem crates/core/src/query/` 返回 11 个 `.rs` 文件(含 mod.rs);`crates/macros/src/linq/` 返回 6 个 `.rs` 文件(含 mod.rs)

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| `builder.rs` imports 缺失或未使用 | 创建后立即 `cargo check -p rust-ef`,按编译器提示逐个修正 |
| 子模块间循环依赖 | 严格分层:`ast` ← `cte`/`window` ← `state` ← `compile` ← `builder` ← `execute_update`/`select`/`source`;`helpers` 独立 |
| `linq/compile.rs` 与 `linq/context.rs` 边界模糊 | `context.rs` 仅含 `LinqCtx`/`FieldKind`/`FieldRef` 及纯字段提取辅助;所有 `compile_*` 函数归 `compile.rs` |
| `extract_field_name_only` 跨模块引用 | 保留在 `expand.rs`,因为仅本模块使用 |
| 可见性错误(pub vs pub(crate)) | `mod.rs` 中 `pub use` 重导出原始可见性;`pub(crate)` 项目用 `pub(crate) use` 显式重导出 |
| QueryBuilder impl 块过大(~1268 行) | 暂不拆分;若未来需要,可考虑 trait 拆分(如 `QueryBuilderCteExt`),但不在本次范围 |
