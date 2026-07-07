# linq.rs 子目录化改造计划

## 概述

延续架构模块化工作,将 `crates/macros/src/linq.rs`(2643 行)拆分为 `linq/` 子目录,共 6 个子模块。`linq/ast.rs` 已在上轮会话中创建,本次需完成剩余 4 个子模块 + mod.rs,并修复 E0027 编译错误。

**目标**:职责清晰、模块化开发,避免单文件堆积大量逻辑,确保架构可演进、可维护、稳定。

## 当前状态分析

### 已完成
- `crates/core/src/query/` 子目录:11 个子模块 + mod.rs ✅(原 query.rs 已删除)
- `crates/macros/src/linq/ast.rs`:184 行,包含所有 AST 类型(LinqInput/QueryInput/ValueInput/LinqClause/HavingExprAst)✅

### 待完成
- `crates/macros/src/linq.rs`:2643 行,仍存在,需拆分后删除
- E0027 编译错误:`linq.rs:1439` — `LinqClause::With` match arm 只解构 4 个字段,但 AST 变体有 6 个字段(漏掉 `recursive: bool` 和 `link: Option<(Expr, Expr)>`)

### linq.rs 精确结构映射(基于 grep 验证)

| 行号范围 | 内容 | 目标模块 |
|---------|------|---------|
| L1-41 | 文件头注释 + imports | 分散到各子模块 |
| L43-216 | AST 类型(已移至 ast.rs) | ✅ ast.rs |
| L218-1239 | Parse 部分(impl Parse + 所有 parse_* 函数 + ValueKind + JoinKind + 辅助函数) | **parse.rs** |
| L1240-1648 | Expand 部分(expand_linq + expand_query + is_true_expr + expand_clauses + expand_join + extract_field_array + extract_field_name_only + expand_value + expand_field_array) | **expand.rs** |
| L1650-1856 | compile_bool_expr + compile_bool_comparison + compile_bool_method | **compile.rs** |
| L1861-1904 | LinqCtx struct + impl + FieldKind + FieldRef | **context.rs** |
| L1909-2404 | compile_expr + compile_not + compile_bool_member + compile_comparison + compile_negated_comparison + compile_contains + SubqueryKind + extract_subquery_closure + compile_subquery_* + compile_not_subquery + compile_in_subquery_* + compile_method + compile_order | **compile.rs** |
| L2410-2542 | extract_field + extract_field_nav + extract_field_ref + type_path_matches + field_const + extract_value | **context.rs** |
| L2543-2643 | compile_having_expr + agg_kind_ident + op_to_ident | **compile.rs** |

### 模块依赖关系(自底向上,无循环)

```
ast.rs      (无依赖,仅 syn 类型)
    ↑
context.rs  (依赖 syn, quote; 提供 LinqCtx/FieldKind/FieldRef/字段提取)
    ↑
compile.rs  (依赖 ast::HavingExprAst, context::*; 提供 compile_* 函数)
    ↑
expand.rs   (依赖 ast::*, compile::{compile_bool_expr, compile_expr, compile_order}, context::{LinqCtx, extract_field, field_const, extract_value})
    ↑
mod.rs      (re-export expand::expand_linq)
```

### E0027 修复方案

**当前代码**(linq.rs L1439-1461,有编译错误):
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

**修复后代码**(在 expand.rs 中实现,完整解构 6 个字段 + 递归 CTE 代码生成):
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

**已验证**:`QueryBuilder::with_recursive_cte_typed` 签名匹配(builder.rs L755-762):
```rust
pub fn with_recursive_cte_typed(
    mut self, name: &str, table: &str,
    link_fk: &str, link_pk: &str,
    where_expr: BoolExpr,
) -> Self
```

## 实施步骤

### Step 1:创建 `linq/context.rs`(无依赖,先建)

**源内容**:linq.rs L1861-1904 + L2410-2542

**包含项目**:
- `LinqCtx<'a>` struct(entity: &'a Type, param: Option<&'a Ident>)
- `impl<'a> LinqCtx<'a>`(含 `single` 构造函数)
- `enum FieldKind`
- `struct FieldRef`
- `fn extract_field(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn extract_field_nav(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn extract_field_ref(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<FieldRef>`
- `fn type_path_matches(path: &syn::Path, entity: &Type) -> bool`
- `fn field_const(entity: &Type, field: &str, kind: FieldKind) -> TokenStream2`
- `fn extract_value(expr: &Expr) -> syn::Result<TokenStream2>`

**imports 头**:
```rust
use proc_macro2::TokenStream2;
use quote::{format_ident, quote};
use syn::{Expr, ExprField, ExprPath, Ident, Member, Type};

use super::ast::*; // 若 LinqCtx 引用到 ast 类型(实际不需要,可省略)
```

**可见性**:所有项 `pub(crate)`,字段 `pub(crate)` 或 `pub`(供 compile.rs/expand.rs 跨模块访问)

### Step 2:创建 `linq/compile.rs`(依赖 ast + context)

**源内容**:linq.rs L1650-1856 + L1909-2404 + L2543-2643

**包含项目**:
- `fn compile_bool_expr(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn compile_bool_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn compile_bool_method(ctx: &LinqCtx<'_>, call: &ExprMethodCall) -> syn::Result<TokenStream2>`
- `fn compile_expr(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn compile_not(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn compile_bool_member(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn compile_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn compile_negated_comparison(ctx: &LinqCtx<'_>, expr: &Expr) -> syn::Result<TokenStream2>`
- `fn compile_contains(...)`
- `enum SubqueryKind` + `impl SubqueryKind`
- `fn extract_subquery_closure(closure: &ExprClosure) -> syn::Result<(Ident, Type)>`
- `fn compile_subquery_parts(...)`
- `fn compile_subquery_method(...)`
- `fn compile_subquery_bool(...)`
- `fn compile_not_subquery(ctx: &LinqCtx<'_>, call: &ExprMethodCall) -> syn::Result<TokenStream2>`
- `fn compile_in_subquery_parts(...)`
- `fn compile_in_subquery_method(...)`
- `fn compile_in_subquery_bool(...)`
- `fn compile_method(ctx: &LinqCtx<'_>, call: &ExprMethodCall) -> syn::Result<TokenStream2>`
- `fn compile_order(ctx: &LinqCtx<'_>, expr: &Expr, descending: bool) -> syn::Result<TokenStream2>`
- `fn compile_having_expr(ast: &HavingExprAst, ctx: &LinqCtx<'_>) -> syn::Result<TokenStream2>`
- `fn agg_kind_ident(agg: &str) -> Ident`
- `fn op_to_ident(op: &str) -> Ident`

**imports 头**:
```rust
use proc_macro2::TokenStream2;
use quote::{format_ident, quote};
use syn::{
    BinOp, Expr, ExprBinary, ExprCall, ExprClosure, ExprField, ExprLit, ExprMethodCall,
    ExprPath, ExprUnary, Ident, Lit, Member, Type, UnOp,
};

use super::ast::HavingExprAst;
use super::context::{
    extract_field, extract_field_ref, extract_value, field_const, FieldKind, FieldRef, LinqCtx,
    type_path_matches,
};
```

**可见性**:所有项 `pub(crate)`,供 expand.rs 调用

### Step 3:创建 `linq/expand.rs`(依赖 ast + compile + context,含 E0027 修复)

**源内容**:linq.rs L1240-1648

**包含项目**:
- `pub fn expand_linq(input: TokenStream) -> TokenStream`(宏入口)
- `fn expand_query(input: &QueryInput) -> syn::Result<TokenStream2>`
- `fn is_true_expr(expr: &Expr) -> bool`
- `fn expand_clauses(input: &QueryInput, entity: &Type) -> syn::Result<TokenStream2>`(**含 E0027 修复**)
- `fn expand_join(params: &[(Ident, Type)], left: &Expr, right: &Expr) -> syn::Result<(TokenStream2, TokenStream2, TokenStream2)>`
- `fn extract_field_array(ctx: &LinqCtx<'_>, fields: &[Expr]) -> syn::Result<TokenStream2>`
- `fn extract_field_name_only(expr: &Expr) -> syn::Result<String>`
- `fn expand_value(input: &ValueInput) -> syn::Result<TokenStream2>`
- `fn expand_field_array(entity: &Type, fields: &[Expr]) -> syn::Result<TokenStream2>`

**imports 头**:
```rust
use proc_macro::TokenStream;
use proc_macro2::TokenStream2;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Expr, ExprField, ExprPath, Ident, Member, Type};

use super::ast::*;
use super::compile::{compile_bool_expr, compile_expr, compile_order};
use super::context::{extract_field, extract_value, field_const, LinqCtx};
```

**关键修改**:`expand_clauses` 的 `LinqClause::With` match arm 使用上述修复方案,完整解构 6 个字段。

### Step 4:创建 `linq/parse.rs`(依赖 ast)

**源内容**:linq.rs L218-1239

**包含项目**:
- `impl Parse for LinqInput`(L222-250)
- `enum ValueKind { Index, Key }`(L252-256)
- `fn parse_value_filter(...)`(L258-275)
- `fn parse_value_index_or_key(...)`(L278-298)
- `fn parse_field_or_tuple(...)`(L301-318)
- `fn parse_query(...)`(L321-427)
- `fn is_source_expr(...)`(L429-437)
- `fn source_entity_type(...)`(L438-466)
- `fn expr_as_entity_type(...)`(L467-477)
- `fn parse_typed_closure(...)`(L478-490)
- `fn parse_closure_with_inference(...)`(L491-513)
- `fn parse_untyped_closure(...)`(L514-521)
- `fn parse_where_rest(...)`(L522-526)
- `fn parse_optional_order(...)`(L527-539)
- `fn parse_order_expr(...)`(L540-552)
- `fn parse_optional_clauses(...)`(L553-570)
- `fn collect_until_semi(...)`(L571-579)
- `fn parse_expr_until_fat_arrow_or_semi(...)`(L580-594)
- `impl Parse for LinqClause`(L600-669)
- `fn parse_include_rest(...)`(L670-690)
- `fn parse_order_by_rest(...)`(L691-712)
- `fn parse_group_by_rest(...)`(L713-718)
- `fn parse_select_rest(...)`(L719-729)
- `fn parse_having_rest(...)`(L730-744)
- `fn parse_window_rest(...)`(L745-823)
- `fn is_window_keyword(...)`(L824-841)
- `fn parse_window_field_list(...)`(L842-864)
- `fn parse_window_order_list(...)`(L865-906)
- `fn parse_window_field_expr(...)`(L907-927)
- `fn is_window_field_boundary(...)`(L928-952)
- `fn parse_with_rest(...)`(L953-1021)— 解析 `recursive`/`link` 关键字
- `fn parse_from_rest(...)`(L1022-1032)
- `fn expr_to_having_ast(...)`(L1033-1076)
- `fn parse_having_compare_from_binary(...)`(L1077-1106)
- `fn parse_agg_call(...)`(L1107-1146)
- `fn bin_op_to_symbol(...)`(L1147-1162)
- `fn parse_set_rest(...)`(L1163-1171)
- `enum JoinKind`(L1172-1179)
- `fn parse_join_rest(...)`(L1180-1232)
- `fn parse_cross_join_rest(...)`(L1233-1239)

**imports 头**:
```rust
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::{
    parse::Parse, BinOp, Expr, ExprBinary, ExprCall, ExprClosure, ExprField, ExprLit,
    ExprMethodCall, ExprPath, ExprUnary, Ident, Lit, Member, Pat, Token, Type, UnOp,
};

use super::ast::*;
```

### Step 5:创建 `linq/mod.rs` + 删除原 `linq.rs`

**mod.rs 内容**:
```rust
//! `linq!()` — compile-time LINQ-to-SQL.
//!
//! Single DSL entry point for all database operations. Supports three
//! syntactic forms: filter closures, multi-clause queries, and value-producing
//! configurations. See `linq!` macro docs in `lib.rs` for usage examples.

mod ast;
mod compile;
mod context;
mod expand;
mod parse;

pub use expand::expand_linq;
```

**操作**:
1. 创建 `crates/macros/src/linq/mod.rs`
2. 删除 `crates/macros/src/linq.rs`(原文件)
3. 验证 `crates/macros/src/lib.rs:6` 的 `mod linq;` 声明无需修改(目录模式自动识别 mod.rs)

### Step 6:编译验证

```powershell
cargo check --workspace
```

预期:无 E0027 错误,无模块解析错误。如有 imports 调整,逐个修复。

### Step 7:测试验证

```powershell
cargo test --workspace
```

预期:所有已有测试通过(特别是 linq! 相关的查询测试、CTE 测试)。

### Step 8:更新 CHANGELOG.md

在 CHANGELOG.md 中追加架构重构条目:
- 拆分 `linq.rs`(2643 行)为 `linq/` 子目录(6 个子模块)
- 修复 E0027 编译错误:实现 `With` 变体完整解构 + 递归 CTE 代码生成
- 模块边界:ast(类型)/ parse(解析)/ context(LinqCtx + 字段提取)/ compile(表达式编译)/ expand(代码生成)/ mod(入口)

## 假设与决策

1. **可见性策略**:所有内部类型/函数使用 `pub(crate)`,仅 `expand_linq` 通过 `pub use` 对 crate 外暴露(实际仅 lib.rs 调用)。与 ast.rs 已有风格一致。

2. **不调整 linq.rs 原有逻辑**:本次仅做物理拆分 + E0027 修复,不重构算法/不优化性能/不改变行为。

3. **不修改 lib.rs**:`mod linq;` 声明对目录模式透明,无需改动。

4. **imports 最小化**:每个子模块只导入自身需要的 syn/quote 项,避免 unused imports 警告。如遇警告,按编译器提示精确调整。

5. **跨模块引用用 `use super::`**:子模块间通过相对路径引用,符合 Rust 模块惯例。

6. **保留原有注释**:section 分隔注释(`// ---`)保留在对应子模块中,作为内部结构标记。

7. **E0027 修复与拆分合并进行**:在创建 expand.rs 时直接写入修复后的 `expand_clauses`,避免二次修改。

## 验证步骤

| 步骤 | 验证内容 | 预期结果 |
|------|---------|---------|
| Step 1-4 | 各子模块文件创建完成 | 文件存在,imports 正确 |
| Step 5 | mod.rs 创建 + linq.rs 删除 | `mod linq;` 解析成功 |
| Step 6 | `cargo check --workspace` | 0 错误,0 警告(或仅 pre-existing 警告) |
| Step 7 | `cargo test --workspace` | 所有测试通过(含 linq! 查询/CTE 测试) |
| Step 8 | CHANGELOG 更新 | 条目完整、准确 |

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| imports 遗漏导致编译错误 | 每个子模块创建后立即 `cargo check`,逐个修复 |
| 跨模块可见性不足 | 统一使用 `pub(crate)`,编译器会提示需提升的项 |
| E0027 修复后 `extract_field_name_only` 调用错误 | 已验证返回 `String`,`quote!` 插值产生字符串字面量,匹配 `&str` 参数 |
| 递归 CTE 测试覆盖不足 | 若无现成测试,至少保证 `cargo check` 通过;测试在后续优先级补齐 |
| 删除 linq.rs 后路径冲突 | 确保先创建 `linq/mod.rs` 再删除 `linq.rs`,避免窗口期编译失败 |
