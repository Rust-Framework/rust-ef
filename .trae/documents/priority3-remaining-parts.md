# Priority 3 剩余部分推进计划

## 概述

继续推进 Priority 3(linq 覆盖补齐 + ITransaction)的剩余工作。基于上一会话的进度:

**已完成(代码层面):**
- ✅ Part 0:ITransaction 封装 —— [transaction.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/transaction.rs) 已创建,db_context.rs 已重构,transaction_ext_tests.rs 已改写
- ✅ Part 3:RIGHT/FULL/CROSS JOIN —— [query.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs) JoinSpec::to_sql 适配 + QueryBuilder 方法 + [linq.rs](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs) 宏子句 + 3 个 JOIN 测试
- ✅ Part 1 代码:SetOperator/SetOpSpec/set_operations 字段/union_internal 等方法/宏子句均已就绪

**待完成:**
1. Part 1 验证:构建验证 + 3 个集合运算测试 + prelude 导出
2. Part 2:递归 CTE
3. Part 4:CASE WHEN 表达式
4. Part 5:UPSERT/MERGE
5. 全局:prelude 导出 + CHANGELOG

详细决策(D1-D8)见原计划 [priority3-linq-coverage-itransaction.md](file:///e:/GitCode/RF/rust-ef/.trae/documents/priority3-linq-coverage-itransaction.md)。

## 当前状态分析(Phase 1 探索结论)

### Part 1(集合运算)—— 代码已就绪,缺验证
- [query.rs#L741-L758](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L741-L758):`SetOperator` 枚举 + `SetOpSpec` 结构已定义
- [query.rs#L1017-L1030](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1017-L1030):`to_sql_with` 已追加集合运算 SQL
- [query.rs#L1042-L1050](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1042-L1050):`all_params` 已合入集合运算参数
- [query.rs#L1869-L1925](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1869-L1925):`union_internal`/`union_all_internal`/`intersect_internal`/`except_internal` 方法已添加
- [linq.rs#L146-L152](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L146-L152):`Union(Expr)`/`UnionAll(Expr)`/`Intersect(Expr)`/`Except(Expr)` 宏变体已添加
- [linq.rs#L1456-L1480](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs#L1456-L1480):`expand_clauses` 代码生成已就绪
- **缺失**:[linq_dsl_tests.rs](file:///e:/GitCode/RF/rust-ef/crates/core/tests/linq_dsl_tests.rs) 无集合运算测试;prelude 未导出 `SetOperator`/`SetOpSpec`
- **缺失**:`compile_sql` 已公开但需确认调用方用法

### Part 2(递归 CTE)—— 未开始
- [query.rs#L721-L739](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L721-L739):`CteSpec` 仅含 name/sql/table/where_expr/params/columns,缺 `is_recursive`/`recursive_link` 字段
- `to_sql_with` 中 CTE 生成逻辑([query.rs#L1001-L1014](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs#L1001-L1014))仅生成非递归 `WITH name AS (...)`,无 RECURSIVE 分支
- QueryBuilder 无 `with_recursive_cte_typed` 方法
- [linq.rs](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs) `LinqClause::With` 无 recursive/link 字段,解析器无 `recursive`/`link` 关键字识别
- [cte_syntax_tests.rs](file:///e:/GitCode/RF/rust-ef/crates/core/tests/cte_syntax_tests.rs) 现有 6 个员工/部门测试数据可复用,需添加递归场景(如员工-经理自引用)

### Part 4(CASE WHEN)—— 未开始
- [query.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs) 无 `ScalarExpr` 枚举,`BoolExpr` 无 `Case` 变体,无 `SelectItem` 结构
- `compile_bool_expr` 无 Case 分支
- `QueryState.projected_columns` 是 `Option<Vec<String>>`,仅支持列名,不支持标量表达式
- [linq.rs](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs) `LinqClause::Select` 仅支持列元组,无 CASE WHEN 解析

### Part 5(UPSERT)—— 未开始
- [entity.rs#L24-L30](file:///e:/GitCode/RF/rust-ef/crates/core/src/entity.rs#L24-L30):`EntityState` 缺 `Upsert` 变体
- [provider.rs#L504-L519](file:///e:/GitCode/RF/rust-ef/crates/core/src/provider.rs#L504-L519):`ISqlGenerator` 缺 `upsert` 方法
- [change_executor.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs):仅有 `execute_inserts`/`execute_updates`/`execute_deletes`,无 `execute_upserts`
- [db_set.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_set.rs):无 `upsert` 方法
- 三个 provider(sqlite/postgres/mysql)的 sql_generator.rs 需实现 `upsert`

### 全局
- [lib.rs#L77-L79](file:///e:/GitCode/RF/rust-ef/crates/core/src/lib.rs#L77-L79):prelude 仅导出 `BoolExpr, CteSpec, LinqSource, ParseFromDb, WindowFuncKind, WindowSpec`,缺 `SetOperator`/`SetOpSpec`(及 Part 4 后的 `ScalarExpr`/`SelectItem`)
- [CHANGELOG.md](file:///e:/GitCode/RF/rust-ef/CHANGELOG.md) 存在,需追加本次变更条目

## 提议变更

### Step 1:Part 1 验证 + prelude 导出

**目标**:验证集合运算代码编译通过,添加测试,导出 prelude。

**修改文件**:[crates/core/src/lib.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/lib.rs)
- L77-79 prelude 的 query 导出追加 `SetOperator, SetOpSpec`:
  ```rust
  pub use crate::query::{
      BoolExpr, CteSpec, LinqSource, ParseFromDb, SetOperator, SetOpSpec,
      WindowFuncKind, WindowSpec,
  };
  ```

**修改文件**:[crates/core/tests/linq_dsl_tests.rs](file:///e:/GitCode/RF/rust-ef/crates/core/tests/linq_dsl_tests.rs)
- 在 JOIN 测试后添加 3 个集合运算测试:
  - `test_union_clause`:两个查询 UNION,验证去重
  - `test_union_all_clause`:UNION ALL 验证不去重
  - `test_except_clause`:EXCEPT 验证差集
- 测试模式:用 `QueryBuilder::compile_sql()` 获取 operand 的 (sql, params),传入 `union_internal` 等

**验证**:
- `cargo build -p rust-ef-macros -p rust-ef` 编译通过
- `cargo test -p rust-ef --test linq_dsl_tests` 全部通过(含 3 个新测试)

### Step 2:Part 2 递归 CTE

**目标**:支持 `linq!(ctx.set::<T>(); with recursive name as |e: T| <anchor_filter>; link e.fk to e.pk; from name)` 语法。

**修改文件**:[crates/core/src/query.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs)

1. `CteSpec` 添加字段(L721-L739):
   ```rust
   pub struct CteSpec {
       // ... 现有字段 ...
       /// 递归 CTE 标记。true 时生成 `WITH RECURSIVE name AS (anchor UNION ALL SELECT ... JOIN name ...)`
       pub is_recursive: bool,
       /// 递归链接:(fk_column, pk_column)。仅 is_recursive=true 时有效。
       pub recursive_link: Option<(String, String)>,
   }
   ```
   注意:`#[non_exhaustive]` 已标注,字段添加不破坏兼容;但现有构造点需补字段。

2. `to_sql_with` CTE 生成逻辑(L1001-L1014)扩展:
   ```rust
   for c in &self.ctes {
       let body = if c.is_recursive {
           // anchor: SELECT * FROM table WHERE <where_expr>
           let anchor = /* 现有 body 生成逻辑 */;
           // recursive: SELECT t.* FROM table t JOIN name ON t.fk = name.pk
           let (fk, pk) = c.recursive_link.clone().expect("recursive CTE needs link");
           format!(
               "{} UNION ALL SELECT t.* FROM {} t JOIN {} ON t.{} = {}.{}",
               anchor, c.table, c.name, fk, c.name, pk
           )
       } else {
           /* 现有 body 生成逻辑 */
       };
       // ... 拼接 WITH [RECURSIVE] ...
   }
   let with_kw = if self.ctes.iter().any(|c| c.is_recursive) { "WITH RECURSIVE" } else { "WITH" };
   sql = format!("{} {} {}", with_kw, cte_parts.join(", "), sql);
   ```

3. QueryBuilder 添加 `with_recursive_cte_typed` 方法(与 `with_cte_typed` 同构,额外接受 link_fk/link_pk):
   ```rust
   pub fn with_recursive_cte_typed<F>(
       mut self,
       name: impl Into<String>,
       link_fk: impl Into<String>,
       link_pk: impl Into<String>,
       filter: F,
   ) -> Self
   where
       F: FnOnce(/* 与 with_cte_typed 相同的签名 */),
   {
       // 构造 CteSpec { is_recursive: true, recursive_link: Some((fk, pk)), ... }
   }
   ```

**修改文件**:[crates/macros/src/linq.rs](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs)

1. `LinqClause::With` 添加字段:
   ```rust
   With {
       name: String,
       entity: Type,
       param: Ident,
       body: Expr,
       recursive: bool,                          // 新增
       link: Option<(Expr, Expr)>,               // 新增:(fk_expr, pk_expr)
   }
   ```

2. 解析器:在 `with` 后检测 `recursive` 关键字;在 body 后检测 `link <fk> to <pk>` 语法:
   ```rust
   // with [recursive] name as |param: Type| <body> [link <fk> to <pk>]
   ```

3. `expand_clauses` 根据 `recursive` 调用 `with_recursive_cte_typed` 或 `with_cte_typed`,从 link 表达式解析出 fk/pk 列名。

**修改文件**:[crates/core/tests/cte_syntax_tests.rs](file:///e:/GitCode/RF/rust-ef/crates/core/tests/cte_syntax_tests.rs)
- 添加自引用实体(如 `RecursiveEmployee` 含 `manager_id` 自引用 FK)
- 添加 2 个测试:
  - `test_recursive_cte_sql_generation`:验证生成 `WITH RECURSIVE ... UNION ALL SELECT t.* FROM ... t JOIN name ON t.manager_id = name.emp_id`
  - `test_recursive_cte_execution`:SQLite 执行递归 CTE,验证层级展开结果

**验证**:
- `cargo build -p rust-ef-macros -p rust-ef` 编译通过
- `cargo test -p rust-ef --test cte_syntax_tests` 全部通过

### Step 3:Part 4 CASE WHEN 表达式

**目标**:支持 `linq!(ctx.set::<T>(); select (b.col, case when b.x > 0 then "high" else "low" end))` 和 WHERE 中 CASE WHEN(D7 限制:仅 WHERE 布尔 + SELECT 标量)。

**修改文件**:[crates/core/src/query.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/query.rs)

1. 新增 `ScalarExpr` 枚举(在 `BoolExpr` 定义附近):
   ```rust
   #[derive(Debug, Clone)]
   pub enum ScalarExpr {
       Column(String),
       Literal(DbValue),
       Case {
           when_clauses: Vec<(BoolExpr, ScalarExpr)>,
           else_clause: Option<Box<ScalarExpr>>,
       },
   }
   ```

2. `BoolExpr` 添加 `Case` 变体(WHERE 上下文):
   ```rust
   pub enum BoolExpr {
       // ... 现有变体 ...
       Case {
           when_clauses: Vec<(BoolExpr, BoolExpr)>,
           else_clause: Option<Box<BoolExpr>>,
       },
   }
   ```
   `total_param_count` 添加 Case 分支递归求和。

3. `compile_bool_expr` 添加 Case 分支:
   ```rust
   BoolExpr::Case { when_clauses, else_clause } => {
       let parts: Vec<String> = when_clauses.iter().map(|(cond, then)| {
           let cond_sql = compile_bool_expr(cond, ...);
           let then_sql = compile_bool_expr(then, ...);
           format!("WHEN {} THEN {}", cond_sql, then_sql)
       }).collect();
       let else_part = else_clause.as_ref().map(|e| format!("ELSE {}", compile_bool_expr(e, ...))).unwrap_or_default();
       format!("CASE {} {} END", parts.join(" "), else_part)
   }
   ```

4. 新增 `compile_scalar_expr` 函数,生成标量表达式 SQL(列名/字面量占位符/CASE WHEN ... THEN scalar ...)。

5. 新增 `SelectItem` 枚举:
   ```rust
   #[derive(Debug, Clone)]
   pub enum SelectItem {
       Column(String),
       Scalar(ScalarExpr),
       Aliased(ScalarExpr, String),
   }
   ```
   `QueryState` 新增 `projected_scalars: Vec<SelectItem>` 字段(保留现有 `projected_columns` 不破坏兼容)。

6. `to_sql_with` 中 SELECT 生成:若 `projected_scalars` 非空,优先使用它生成 SELECT 列表。

**修改文件**:[crates/macros/src/linq.rs](file:///e:/GitCode/RF/rust-ef/crates/macros/src/linq.rs)

1. `LinqClause::Select` 扩展:支持 `case when <cond> then <val> [when ... else <val>] end` 作为 select 项。
2. 新增 `case when` 表达式解析器(在 select 元组解析中识别 `case` 关键字),生成 `ScalarExpr::Case`。
3. `expand_clauses` 调用新方法 `select_scalar_internal` 或扩展 `select_internal` 接受 `SelectItem`。

**修改文件**:[crates/core/tests/linq_dsl_tests.rs](file:///e:/GitCode/RF/rust-ef/crates/core/tests/linq_dsl_tests.rs)
- 添加 2 个测试:
  - `test_case_when_in_select`:SELECT 中 CASE WHEN 标量分类
  - `test_case_when_in_where`:WHERE 中 CASE WHEN 布尔判断

**验证**:
- `cargo build -p rust-ef-macros -p rust-ef` 编译通过
- `cargo test -p rust-ef --test linq_dsl_tests` 全部通过

### Step 4:Part 5 UPSERT/MERGE

**目标**:支持 `ctx.set::<Blog>().upsert(blog)` API(D8:仅 API,不进 linq! 宏)。

**修改文件**:[crates/core/src/entity.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/entity.rs#L24-L30)
- `EntityState` 添加 `Upsert` 变体

**修改文件**:[crates/core/src/db_set.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_set.rs)
- `DbSet<T>` 添加 `pub fn upsert(&mut self, entity: T)`,设置 `EntityState::Upsert`(与 `add` 同构,仅状态不同)

**修改文件**:[crates/core/src/provider.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/provider.rs#L504-L519)
- `ISqlGenerator` 添加方法:
  ```rust
  fn upsert(
      &self,
      table: &str,
      insert_cols: &[&str],
      conflict_cols: &[&str],
      update_cols: &[&str],
  ) -> String;
  ```

**修改文件**:三个 provider 的 sql_generator.rs
- [sqlite/sql_generator.rs](file:///e:/GitCode/RF/rust-ef/crates/sqlite/src/sql_generator.rs):`INSERT INTO ... (...) VALUES (...) ON CONFLICT (conflict_cols) DO UPDATE SET col = EXCLUDED.col, ...`
- [postgres/sql_generator.rs](file:///e:/GitCode/RF/rust-ef/crates/postgres/src/sql_generator.rs):同 SQLite 语法(PostgreSQL 原生支持 ON CONFLICT)
- [mysql/sql_generator.rs](file:///e:/GitCode/RF/rust-ef/crates/mysql/src/sql_generator.rs):`INSERT INTO ... (...) VALUES (...) ON DUPLICATE KEY UPDATE col = VALUES(col), ...`(MySQL 无冲突列,conflict_cols 参数忽略)

**修改文件**:[crates/core/src/change_executor.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs)
- 添加 `pub async fn execute_upserts<E, F>(...)` 函数,与 `execute_inserts` 同构但:
  - 使用 `gen.upsert(table, insert_cols, conflict_cols, update_cols)` 生成 SQL
  - conflict_cols:实体的主键列
  - update_cols:除主键外的所有列(或所有列,由 provider 决定是否排除冲突列)

**修改文件**:[crates/core/src/db_context.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/db_context.rs)
- `save_changes` 中区分 `EntityState::Upsert`,调用 `execute_upserts`

**修改文件**:[crates/core/tests/sqlite_crud_tests.rs](file:///e:/GitCode/RF/rust-ef/crates/core/tests/sqlite_crud_tests.rs)
- 添加 2 个测试:
  - `test_upsert_insert_when_not_exists`:upsert 新实体,验证插入
  - `test_upsert_update_when_exists`:upsert 已有主键实体,验证更新

**验证**:
- `cargo build --workspace` 编译通过
- `cargo test -p rust-ef --test sqlite_crud_tests` 全部通过

### Step 5:全局验证 + CHANGELOG

**修改文件**:[crates/core/src/lib.rs](file:///e:/GitCode/RF/rust-ef/crates/core/src/lib.rs)
- prelude 导出补全:
  ```rust
  pub use crate::query::{
      BoolExpr, CteSpec, LinqSource, ParseFromDb, ScalarExpr, SelectItem,
      SetOperator, SetOpSpec, WindowFuncKind, WindowSpec,
  };
  ```

**修改文件**:[CHANGELOG.md](file:///e:/GitCode/RF/rust-ef/CHANGELOG.md)
- 追加本次变更条目(Part 0-5 全部)

**验证**:
- `cargo build --workspace` 全部编译通过
- `cargo test --workspace` 全部通过(忽略环境相关的 PostgreSQL/MySQL 测试失败)

## 假设与决策

延续原计划 D1-D8,本次无新增决策:
- **D5**:集合运算 operand 不含 ORDER BY/LIMIT
- **D7**:CASE WHEN 仅支持 WHERE 布尔 + SELECT 标量,不支持 GROUP BY/HAVING/ORDER BY 中的 CASE
- **D8**:UPSERT 仅 API,不进 linq! 宏
- **递归 CTE anchor**:filter(在 with 子句前的 where)作为 anchor WHERE;body 解析为 link 表达式。若 anchor 无 filter,生成 `SELECT * FROM table`(无 WHERE)

## 实施顺序

1. **Step 1**:Part 1 验证 + prelude 导出 → verify: linq_dsl_tests 通过
2. **Step 2**:Part 2 递归 CTE → verify: cte_syntax_tests 通过
3. **Step 3**:Part 4 CASE WHEN → verify: linq_dsl_tests 通过
4. **Step 4**:Part 5 UPSERT → verify: sqlite_crud_tests 通过
5. **Step 5**:全局验证 + CHANGELOG → verify: cargo build/test --workspace 通过

## 任务清单

- [ ] Step 1: Part 1 验证 —— prelude 导出 SetOperator/SetOpSpec + 3 个集合运算测试
- [ ] Step 2: Part 2 递归 CTE —— CteSpec 递归字段 + to_sql_with RECURSIVE + with_recursive_cte_typed + linq! with recursive link + 2 个测试
- [ ] Step 3: Part 4 CASE WHEN —— ScalarExpr + BoolExpr::Case + compile_scalar_expr + SelectItem + linq! case when + 2 个测试
- [ ] Step 4: Part 5 UPSERT —— EntityState::Upsert + DbSet::upsert + ISqlGenerator::upsert + 3 provider 实现 + execute_upserts + save_changes 分发 + 2 个测试
- [ ] Step 5: 全局验证 + CHANGELOG + prelude 补全导出
