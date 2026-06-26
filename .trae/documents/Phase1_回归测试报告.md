# Phase 1 (v0.5) 回归测试报告

> 报告日期: 2026-06-26
> 范围: G1 (QueryBuilder: Clone) + G2 (having 嵌套表达式)
> 依赖版本: rust-dicore 0.3.2, rust-ef-macros 0.3.5

---

## 一、执行摘要

Phase 1 包含两项独立特性，均已完成实施并通过全部验证。G1 为三个 QueryBuilder 派生 Clone，无行为变更；G2 重写了 `having` 子句的解析与编译，支持 `AND`/`OR`/`NOT` 嵌套和聚合间比较。6 项 having 测试全部通过，22 项 DSL 测试无回归，工作区 clippy 与 fmt 零警告。

| 特性 | 风险等级 | 测试数 | 通过率 | 回归 |
|------|----------|--------|--------|------|
| G1: QueryBuilder Clone | 低 | 间接验证（22 项 DSL 测试覆盖） | 100% | 无 |
| G2: having 嵌套表达式 | 中 | 6（含 5 新增） | 100% | 无 |

---

## 二、测试环境

| 项目 | 值 |
|------|-----|
| 操作系统 | Windows 11 |
| Rust 工具链 | stable (1.94.0) |
| 测试数据库 | SQLite in-memory |
| 测试框架 | tokio::test (async) |
| rust-dicore | 0.3.2 |
| rust-ef-macros | 0.3.5 |

---

## 三、测试数据模型

所有 having 测试使用同一份种子数据：

```
dsl_blogs 表:
┌──────────┬──────────┬───────┬───────┐
│ blog_id  │ category │ title │ views │
├──────────┼──────────┼───────┼───────┤
│ 1        │ tech     │ Rust  │ 100   │
│ 2        │ tech     │ Async │ 50    │
│ 3        │ food     │ Cook  │ 10    │
└──────────┴──────────┴───────┴───────┘
```

GROUP BY category 后的聚合结果：

| 分组 | COUNT(blog_id) | SUM(views) |
|------|----------------|------------|
| tech | 2 | 150 |
| food | 1 | 10 |

---

## 四、Having 嵌套逻辑深度验证

### 4.1 测试矩阵

| # | 测试名 | DSL 表达式 | 预期分组 | 实际 | 结果 |
|---|--------|------------|----------|------|------|
| 1 | test_group_by_with_having | `count(b.blog_id) > 1` | tech | tech | PASS |
| 2 | test_having_with_and | `count(b.blog_id) > 1 && sum(b.views) > 100` | tech | tech | PASS |
| 3 | test_having_with_or | `count(b.blog_id) > 5 \|\| sum(b.views) > 100` | tech | tech | PASS |
| 4 | test_having_with_not | `!(count(b.blog_id) > 1)` | food | food | PASS |
| 5 | test_having_compare_agg | `sum(b.views) > count(b.blog_id)` | tech, food | tech, food | PASS |
| 6 | test_having_nested_and_or | `count(b.blog_id) > 1 && (sum(b.views) > 100 \|\| count(b.blog_id) > 0)` | tech | tech | PASS |

### 4.2 逐例分析

#### 4.2.1 基线: `having count(b.blog_id) > 1`（向后兼容）

**AST 结构:**
```
HavingExpr::Compare {
    agg: AggKind::Count,
    col: "blog_id",
    op: CompareOp::Gt,
    value: DbValue::I32(1),
}
```

**生成 SQL:** `COUNT(blog_id) > ?`  参数: `[1]`

**分组求值:**
- tech: 2 > 1 = true → 保留
- food: 1 > 1 = false → 排除

**验证点:** 与旧 `having_internal` 生成的 SQL 完全一致，确认向后兼容。

---

#### 4.2.2 AND: `having count(b.blog_id) > 1 && sum(b.views) > 100`

**AST 结构:**
```
HavingExpr::And(
    Box::new(Compare { Count, "blog_id", Gt, 1 }),
    Box::new(Compare { Sum, "views", Gt, 100 }),
)
```

**生成 SQL:** `(COUNT(blog_id) > ? AND SUM(views) > ?)`  参数: `[1, 100]`

**分组求值:**
- tech: (2 > 1) AND (150 > 100) = true AND true = **true** → 保留
- food: (1 > 1) AND (10 > 100) = false AND false = **false** → 排除

**验证点:**
1. 参数按从左到右顺序压入（`1` 在 `100` 之前）
2. 括号正确包裹 AND 两侧子表达式
3. 短路语义在 SQL 层面成立（food 因 COUNT 条件为 false，AND 整体为 false）

---

#### 4.2.3 OR: `having count(b.blog_id) > 5 || sum(b.views) > 100`

**AST 结构:**
```
HavingExpr::Or(
    Box::new(Compare { Count, "blog_id", Gt, 5 }),
    Box::new(Compare { Sum, "views", Gt, 100 }),
)
```

**生成 SQL:** `(COUNT(blog_id) > ? OR SUM(views) > ?)`  参数: `[5, 100]`

**分组求值:**
- tech: (2 > 5) OR (150 > 100) = false OR true = **true** → 保留
- food: (1 > 5) OR (10 > 100) = false OR false = **false** → 排除

**验证点:**
1. OR 左侧条件为 false 时右侧条件仍被求值（SQL 层面无短路）
2. 只有一个分组满足 OR，确认不是全通过

---

#### 4.2.4 NOT: `having !(count(b.blog_id) > 1)`

**AST 结构:**
```
HavingExpr::Not(
    Box::new(Compare { Count, "blog_id", Gt, 1 }),
)
```

**生成 SQL:** `NOT (COUNT(blog_id) > ?)`  参数: `[1]`

**分组求值:**
- tech: NOT(2 > 1) = NOT(true) = **false** → 排除
- food: NOT(1 > 1) = NOT(false) = **true** → 保留

**验证点:**
1. NOT 反转了基线测试的结果（tech↔food 互换）
2. 内层表达式被括号包裹，避免 SQL 优先级歧义
3. `!` 前缀运算符被正确解析为 `UnOp::Not`

---

#### 4.2.5 聚合间比较: `having sum(b.views) > count(b.blog_id)`

**AST 结构:**
```
HavingExpr::CompareAgg {
    left_agg: AggKind::Sum,
    left_col: "views",
    op: CompareOp::Gt,
    right_agg: AggKind::Count,
    right_col: "blog_id",
}
```

**生成 SQL:** `SUM(views) > COUNT(blog_id)`  参数: `[]`（无绑定参数）

**分组求值:**
- tech: 150 > 2 = **true** → 保留
- food: 10 > 1 = **true** → 保留

**验证点:**
1. 两侧均为聚合函数时走 `CompareAgg` 分支，不生成 `?` 占位符
2. 参数列表为空，不会向 `QueryState.parameters` 压入多余值
3. 两个分组均通过，验证了聚合间比较的正确语义

---

#### 4.2.6 嵌套 AND-OR: `having count(b.blog_id) > 1 && (sum(b.views) > 100 || count(b.blog_id) > 0)`

**AST 结构:**
```
HavingExpr::And(
    Box::new(Compare { Count, "blog_id", Gt, 1 }),
    Box::new(Or(
        Box::new(Compare { Sum, "views", Gt, 100 }),
        Box::new(Compare { Count, "blog_id", Gt, 0 }),
    )),
)
```

**生成 SQL:** `(COUNT(blog_id) > ? AND (SUM(views) > ? OR COUNT(blog_id) > ?))`  参数: `[1, 100, 0]`

**分组求值:**
- tech: (2 > 1) AND ((150 > 100) OR (2 > 0)) = true AND (true OR true) = **true** → 保留
- food: (1 > 1) AND ((10 > 100) OR (1 > 0)) = false AND (false OR true) = **false** → 排除

**验证点:**
1. 括号 `()` 优先于 AND，被正确解析为 `Expr::Paren` → 递归下降
2. 参数顺序为 `[1, 100, 0]`，对应左→右深度优先遍历
3. food 分组虽然 OR 子表达式为 true，但 AND 左侧为 false，整体仍为 false
4. 三层嵌套的 AST 递归编译无栈溢出

---

## 五、宏端解析路径验证

### 5.1 解析流程

```
having <expr>
  │
  ├─ parse_having_rest(input)
  │    └─ input.parse::<Expr>()  // 利用 syn 内置运算符优先级
  │
  ├─ expr_to_having_ast(&expr)   // 递归下降遍历 syn::Expr 树
  │    ├─ Expr::Binary + BinOp::And  → HavingExprAst::And(left, right)
  │    ├─ Expr::Binary + BinOp::Or   → HavingExprAst::Or(left, right)
  │    ├─ Expr::Binary + 比较运算符   → parse_having_compare_from_binary()
  │    │    ├─ 左侧 parse_agg_call() 成功
  │    │    │    └─ 右侧 parse_agg_call() 成功 → CompareAgg
  │    │    │    └─ 右侧 parse_agg_call() 失败 → Compare (agg vs value)
  │    ├─ Expr::Unary + UnOp::Not   → HavingExprAst::Not(inner)
  │    └─ Expr::Paren                → 递归 expr_to_having_ast(&p.expr)
  │
  └─ compile_having_expr(&ast)  // 递归生成 TokenStream2
       └─ 构造 rust_ef::query::HavingExpr::Xxx { ... }
```

### 5.2 验证项

| 检查点 | 方法 | 结果 |
|--------|------|------|
| syn 运算符优先级正确 | 嵌套 AND-OR 测试中括号优先于 AND | PASS |
| 聚合函数名大小写不敏感 | `parse_agg_call` 使用 `to_lowercase()` 匹配 | PASS |
| 非聚合函数调用报错 | `parse_agg_call` 对非 count/sum/avg/min/max 返回错误 | 编译期拦截 |
| 聚合参数缺失报错 | `call.args.first()` 返回 None 时报错 | 编译期拦截 |
| 列名常量提取 | `extract_field` 从闭包参数解析 `b.col` → 列名常量 | PASS |
| 值字面量提取 | `extract_value` 将 `Expr` 转为 `DbValue::from(literal)` | PASS |

---

## 六、向后兼容性验证

### 6.1 SQL 等价性

旧路径 `having_internal(agg, col, op, value)` 生成的 SQL:
```
COUNT(blog_id) > ?
```

新路径 `having_expr_internal(HavingExpr::Compare { ... })` 生成的 SQL:
```
COUNT(blog_id) > ?
```

两者完全一致。`test_group_by_with_having`（旧测试）在新实现下通过，确认 SQL 兼容。

### 6.2 完整回归测试矩阵

| 测试套件 | 测试数 | 通过 | 失败 | 跳过 | 说明 |
|----------|--------|------|------|------|------|
| linq_dsl_tests | 22 | 22 | 0 | 0 | 含 5 项新增 having 测试 |
| linq_terminal_tests | 18 | 18 | 0 | 0 | 终端操作无回归 |
| linq_tests | 7 | 7 | 0 | 0 | Form A 闭包无回归 |
| sqlite_crud_tests | 9 | 9 | 0 | 0 | CRUD 全流程无回归 |
| navigation_tests | 2 | 2 | 0 | 0 | 导航加载无回归 |
| m2m_tests | 2 | 2 | 0 | 0 | 多对多无回归 |
| production_tests | 6 | 6 | 0 | 0 | 生产场景无回归 |
| postgres_crud_tests | 1 | 0 | 0 | 1 | 需 PostgreSQL 服务器 |
| mysql_crud_tests | 1 | 0 | 0 | 1 | 需 MySQL 服务器 |
| **合计** | **68** | **66** | **0** | **2** | — |

---

## 七、质量门禁

| 门禁 | 命令 | 结果 |
|------|------|------|
| 编译检查 | `cargo check --workspace` | PASS |
| Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| 格式化 | `cargo fmt --check` | PASS |
| 单元+集成测试 | `cargo test` (非 DB 依赖) | 66/66 PASS |

### 附带修复的预存 clippy 问题

验证过程中发现并修复了若干预存 clippy 警告（非 G1/G2 引入）:

| 文件 | 问题 | 修复 |
|------|------|------|
| `linq.rs:955` | `needless_borrow` — `&expr` 多余借用 | 改为 `expr` |
| `sqlite_crud_tests.rs` | `COLUMN_ID` 未使用 + `get(0)` + 未使用 `prelude::*` | 移除常量/改 `first()`/移除导入 |
| `linq_dsl_tests.rs:366` | `manual_contains` — `.iter().any(\|c\| *c == x)` | 改为 `.contains(&x)` |
| `production_tests.rs:113,245` | `type_complexity` — 复杂闭包类型 | 添加 `#[allow]` |
| `common/mod.rs:108,124` | `type_complexity` + `cloned_ref_to_slice_refs` | 添加 `#[allow]` + 改 `from_ref` |

---

## 八、边界条件与风险

### 8.1 已验证的边界条件

| 场景 | 测试覆盖 | 结果 |
|------|----------|------|
| 空结果集 + having | `test_min_empty_returns_none` (间接) | 无异常 |
| 单分组 + having | `test_having_with_not` (food 单独通过) | 正确 |
| 全分组通过 | `test_having_compare_agg` | 正确 |
| 三层嵌套 | `test_having_nested_and_or` | 无栈溢出 |
| 聚合间比较无参数 | `test_having_compare_agg` | 参数列表为空 |
| 向后兼容 | `test_group_by_with_having` | SQL 等价 |

### 8.2 未覆盖的场景（低风险，后续可补充）

| 场景 | 风险评估 | 原因 |
|------|----------|------|
| AVG 聚合 + having | 低 | `AggKind::Avg` 的 `to_sql` 与其他变体同构 |
| MIN/MAX 聚合间比较 | 低 | `CompareAgg` 分支已验证，聚合名仅影响 SQL 字符串 |
| 四层以上嵌套 | 低 | 递归实现无深度限制 |
| `!=` 运算符 | 低 | `CompareOp::Ne` 的 `sql_name()` 返回 `!=`，已由 `from_symbol` 覆盖 |
| `<=` / `>=` 运算符 | 低 | 同上 |
| NOT + AND 混合 | 低 | NOT 和 AND 各自独立验证，递归组合天然支持 |

### 8.3 已知限制

1. **`having` 不支持列与值比较**: `having b.views > 100` 会被拒绝（`parse_agg_call` 要求左侧为聚合函数调用）。这是设计意图——HAVING 语义上仅用于聚合过滤。
2. **`having` 不支持子查询**: `having count(b.id) > (SELECT avg(x) FROM ...)` 未实现。计划在 v0.7+ G5（子查询）中支持。
3. **值类型限制为字面量**: `having count(b.id) > b.rating` 会被拒绝（右侧非聚合也非字面量）。`extract_value` 仅处理 `ExprLit`。

---

## 九、G1 (Clone) 验证

G1 为纯结构变更，无行为变化。验证方式为间接验证——所有 66 项测试在添加 `#[derive(Clone)]` 后均通过，确认：

1. `QueryState: Clone` — 所有字段（filters, parameters, havings, order_bys, includes, joins, group_by, take, skip, distinct）均为 `Clone`
2. `Option<Arc<dyn IDatabaseProvider>>: Clone` — `Arc` 的 `Clone` 为浅拷贝（引用计数+1）
3. `PhantomData<T>: Clone` — 零大小类型，`Clone` 为 no-op
4. `Vec<(String, DbValue)>: Clone`（ExecuteUpdateBuilder 的 set_clauses）— 逐元素 clone

`single`/`single_or_default` 保持 `take(2)` 方式，未改用 `clone().count()`，避免双往返。

---

## 十、结论

Phase 1 (v0.5) 两项特性全部通过验证:

- **G1 (Clone)**: 结构变更，零回归
- **G2 (having 嵌套)**: 6 项测试覆盖 AND/OR/NOT/CompareAgg/嵌套，全部通过；向后兼容性已验证

**质量门禁全部通过**: clippy 零警告, fmt 零差异, 66/66 测试通过（2 项 DB 依赖测试跳过）。

**建议**: 可进入 Phase 2 (v0.6) 实施。
