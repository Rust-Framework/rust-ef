# Priority 3: linq 覆盖补齐 + ITransaction 封装

## 概述

按优先级继续推进 REF 框架的完善工作。在 Priority 1(元数据缓存)和 Priority 2(事务接口扩展)已完成的基础上,本次推进两件事:

1. **Part 0:ITransaction 封装** —— 将 Priority 2 散落在 `DbContext` 上的事务方法抽象为 `ITransaction` trait,提供独立的事务句柄,支持 `use(...)` 作用域模式。
2. **Part 1-5:linq 覆盖补齐** —— 补全 `linq!` 宏和查询构建器的 5 项关键缺口:
   - 集合运算(UNION/INTERSECT/EXCEPT)
   - 递归 CTE
   - RIGHT/FULL/CROSS JOIN
   - CASE WHEN 表达式
   - UPSERT/MERGE

## 当前状态分析

### Priority 2 事务接口现状(需重构)

文件:`crates/core/src/db_context.rs`

- L343:`ambient_transaction: Option<Box<dyn IAsyncConnection>>` —— 字段类型直接持有连接,无事务句柄抽象
- L551:`begin_transaction(&mut self) -> EFResult<()>` —— 返回 `()`,无法暴露事务句柄
- L565/L575:`commit_transaction` / `rollback_transaction` —— DbContext 上的方法,非事务句柄方法
- L616-677:`save_changes()` —— 内部 `TxnSource` 枚举(Ambient/Managed)区分接管/自管事务
- L705-723:`use_transaction<F, Fut, R>(&self, f)` —— **简单版**,自建连接,不与 ambient 协同
- L727+:`create_savepoint` / `release_savepoint` / `rollback_to_savepoint` / `set_transaction_isolation` —— DbContext 上的代理方法

**问题**:事务能力散落在 DbContext 上,句柄与上下文耦合,无法独立传递事务;`use_transaction` 不与 ambient 协同,无法在同一事务内多次 `save_changes`。

### linq! 宏覆盖现状(需补齐)

文件:`crates/macros/src/linq.rs`(约 2480 行)

- L89-154:`LinqClause` 枚举 —— 现有 Include/OrderBy/GroupBy/Select/HavingExpr/Sum/Avg/Min/Max/Count/Distinct/Set/InnerJoin/LeftJoin/ExecuteUpdate/Take/Skip/Window/With/From
- 缺失:`Union` / `Intersect` / `Except`(集合运算)、`RightJoin` / `FullJoin` / `CrossJoin`、`Case`、递归 `With`

文件:`crates/core/src/query.rs`

- L690-787:`CteSpec`(name, sql, table, where_expr, params, columns)—— 缺 `is_recursive` / `recursive_link`
- L690-787:`QueryState` —— 缺 `set_operations` 字段
- `JoinSpec`(join_type: String, table: String, on_clause: String)—— String 类型允许任意 join type,但宏层未暴露
- `BoolExpr` 枚举(8 变体:Filter/Raw/And/Or/Not/Exists/NotExists/InSubquery/NotInSubquery)—— 纯布尔树,无标量支持,CASE WHEN 需扩展

文件:`crates/core/src/provider.rs` L504-534

- `ISqlGenerator` trait —— 缺 `upsert` 方法

文件:`crates/core/src/entity.rs` L24-30

- `EntityState` 枚举(Detached/Added/Unchanged/Modified/Deleted)—— 缺 `Upsert` 变体

文件:`crates/core/src/change_executor.rs`

- 仅 `execute_inserts` / `execute_updates` / `execute_deletes`,无 `execute_upserts`

## 提议变更

### Part 0:ITransaction 封装(重构 Priority 2)

**目标**:将事务能力从 DbContext 解耦为独立 `ITransaction` 句柄,支持 `use(...)` 作用域模式。

**新建文件**:`crates/core/src/transaction.rs`

```rust
use crate::error::EFResult;
use crate::provider::{IAsyncConnection, IsolationLevel};
use std::future::Future;
use std::pin::Pin;

/// 事务句柄抽象。封装 Priority 2 的底层连接事务方法。
///
/// - `commit` / `rollback` 消费 `self: Box<Self>`,防止提交后误用(D1)。
/// - `connection()` 暴露底层连接,供 `save_changes` 复用事务。
/// - `use_transaction` 在闭包内注册 ambient,使 `save_changes` 自动复用(D3)。
pub trait ITransaction: Send {
    /// 提交事务并消费句柄。
    fn commit(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>>;

    /// 回滚事务并消费句柄。
    fn rollback(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>>;

    /// 创建保存点。
    fn create_point(
        &mut self,
        name: &str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>>;

    /// 释放(提交)保存点。
    fn release_point(
        &mut self,
        name: &str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>>;

    /// 回滚到保存点,保留外层事务。
    fn rollback_point(
        &mut self,
        name: &str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>>;

    /// 设置事务隔离级别。
    fn set_isolation(
        &mut self,
        level: IsolationLevel,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>>;

    /// 暴露底层连接,供 `save_changes` 等内部复用事务。
    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send);
}

/// 默认实现:包装 `IAsyncConnection` 提供事务句柄语义。
pub struct DbTransaction {
    conn: Option<Box<dyn IAsyncConnection>>,
}

impl DbTransaction {
    pub fn new(conn: Box<dyn IAsyncConnection>) -> Self {
        Self { conn: Some(conn) }
    }
}

impl ITransaction for DbTransaction {
    fn commit(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>> {
        Box::pin(async move {
            let mut conn = self.conn.expect("transaction already consumed");
            conn.commit_transaction().await
        })
    }

    fn rollback(self: Box<Self>) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + 'static>> {
        Box::pin(async move {
            let mut conn = self.conn.expect("transaction already consumed");
            conn.rollback_transaction().await
        })
    }

    fn create_point(
        &mut self,
        name: &str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::Transaction("transaction consumed".into()))?
                .create_savepoint(name)
                .await
        })
    }

    fn release_point(
        &mut self,
        name: &str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::Transaction("transaction consumed".into()))?
                .release_savepoint(name)
                .await
        })
    }

    fn rollback_point(
        &mut self,
        name: &str,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::Transaction("transaction consumed".into()))?
                .rollback_to_savepoint(name)
                .await
        })
    }

    fn set_isolation(
        &mut self,
        level: IsolationLevel,
    ) -> Pin<Box<dyn Future<Output = EFResult<()>> + Send + '_>> {
        Box::pin(async move {
            self.conn
                .as_mut()
                .ok_or_else(|| crate::error::EFError::Transaction("transaction consumed".into()))?
                .set_transaction_isolation(level)
                .await
        })
    }

    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send) {
        self.conn
            .as_mut()
            .expect("transaction already consumed")
            .as_mut()
    }
}
```

**修改文件**:`crates/core/src/lib.rs`

- 添加 `pub mod transaction;`
- 在 prelude 中导出:`pub use crate::transaction::{ITransaction, DbTransaction};`

**修改文件**:`crates/core/src/db_context.rs`

1. L343 字段类型变更:
   ```rust
   ambient_transaction: Option<Box<dyn ITransaction>>,
   ```

2. L551 `begin_transaction` 返回句柄(不注册 ambient,见 D3):
   ```rust
   pub async fn begin_transaction(&mut self) -> EFResult<Box<dyn ITransaction>> {
       let mut conn = self.provider.get_connection().await?;
       conn.begin_transaction().await?;
       Ok(Box::new(DbTransaction::new(conn)))
   }
   ```
   **注意**:返回的句柄不注册到 ambient;调用方需通过 `use_transaction` 才能让 `save_changes` 复用。

3. 删除 `commit_transaction` / `rollback_transaction` / `create_savepoint` / `release_savepoint` / `rollback_to_savepoint` / `set_transaction_isolation` —— 这些方法已在 `ITransaction` 句柄上提供(`create_point` / `release_point` / `rollback_point` / `set_isolation`)。

4. L705 `use_transaction` 改为 ambient-aware 版本:
   ```rust
   pub async fn use_transaction<F, Fut, R>(&mut self, f: F) -> EFResult<R>
   where
       for<'a> F: FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = EFResult<R>> + Send + 'a>>,
       R: Send + 'static,
   {
       let mut conn = self.provider.get_connection().await?;
       conn.begin_transaction().await?;
       self.ambient_transaction = Some(Box::new(DbTransaction::new(conn)));
       let result = f(self).await;
       let txn = self.ambient_transaction.take();
       match (result, txn) {
           (Ok(r), Some(txn)) => { txn.commit().await?; Ok(r) }
           (Err(e), Some(txn)) => { let _ = txn.rollback().await; Err(e) }
           (_, None) => unreachable!("ambient_transaction set above"),
       }
   }
   ```
   调用方样板(D4 接受):
   ```rust
   ctx.use_transaction(|ctx| Box::pin(async move {
       ctx.set::<Blog>().add(blog);
       ctx.save_changes().await?;
       Ok(())
   })).await?;
   ```

5. L587 `save_changes` 适配:使用 `ITransaction::connection()` 获取底层连接:
   ```rust
   enum TxnSource {
       Ambient(Box<dyn ITransaction>),
       Managed(Box<dyn IAsyncConnection>),
   }
   let mut txn = match self.ambient_transaction.take() {
       Some(t) => TxnSource::Ambient(t),
       None => {
           let mut c = self.provider.get_connection().await?;
           c.begin_transaction().await?;
           TxnSource::Managed(c)
       }
   };
   // ... 在循环中:
   let conn_ref: &mut dyn IAsyncConnection = match &mut txn {
       TxnSource::Ambient(t) => t.connection(),
       TxnSource::Managed(c) => c.as_mut(),
   };
   ```
   错误路径 Managed 回滚,Ambient 还原;成功路径 Managed 提交,Ambient 还原。

**修改文件**:`crates/core/tests/transaction_ext_tests.rs`

- 将 `ctx.begin_transaction().await?; ... ctx.commit_transaction().await?;` 改为:
  ```rust
  let txn = ctx.begin_transaction().await?;
  // ... save_changes
  txn.commit().await?;
  ```
- 删除对 `ctx.create_savepoint` 等的调用,改为 `txn.create_point(...)` 等。
- 添加 4 个 `use_transaction` 测试(成功提交、错误回滚、保存点、隔离级别)。

### Part 3:RIGHT/FULL/CROSS JOIN(优先做,简单)

**目标**:补齐 JOIN 类型,JoinSpec 已是 String,只需宏暴露 + SQL 生成适配。

**修改文件**:`crates/core/src/query.rs`

- `JoinSpec::to_sql()` 已用 `format!("{} JOIN {} ON {}", self.join_type, ...)` —— CROSS JOIN 无 ON 子句,需特殊处理:
  ```rust
  pub fn to_sql(&self) -> String {
      if self.join_type == "CROSS" {
          format!("CROSS JOIN {}", self.table)
      } else {
          format!("{} JOIN {} ON {}", self.join_type, self.table, self.on_clause)
      }
  }
  ```

**修改文件**:`crates/core/src/query.rs` QueryBuilder

- 添加 `right_join_internal` / `full_join_internal` / `cross_join_internal` 方法,与 `inner_join_internal` 同构(仅 join_type 字符串不同)。
- CROSS JOIN 无 ON 子句,接受空 on_clause。

**修改文件**:`crates/macros/src/linq.rs`

- `LinqClause` 添加 `RightJoin` / `FullJoin` / `CrossJoin` 变体(与 `InnerJoin` 同构,CrossJoin 不含 on 条件)。
- `parse_clauses` 添加对应解析分支。
- `expand_clauses` 添加对应代码生成,调用 `right_join` / `full_join` / `cross_join` 方法。

**修改文件**:`crates/core/src/query.rs` QueryBuilder 公共 API

- 添加 `pub fn right_join<E, F>` / `pub fn full_join<E, F>` / `pub fn cross_join<E>` 方法。

**注意**:MySQL 不支持 FULL OUTER JOIN(D6),在 MySQL provider 的 `to_sql_with` 中如果检测到 FULL JOIN 需 panic 或返回错误(暂不模拟)。

### Part 1:集合运算 UNION/INTERSECT/EXCEPT

**目标**:支持 `linq!(ctx.set::<Blog>(); union <subquery>; intersect <subquery>; except <subquery>)` 语法。

**修改文件**:`crates/core/src/query.rs`

1. 新增枚举:
   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum SetOperator {
       Union,
       UnionAll,
       Intersect,
       Except,
   }

   #[derive(Debug, Clone)]
   pub struct SetOpSpec {
       pub operator: SetOperator,
       pub operand_sql: String,
       pub operand_params: Vec<DbValue>,
   }
   ```

2. `QueryState` 添加字段:
   ```rust
   pub(crate) set_operations: Vec<SetOpSpec>,
   ```

3. `to_sql_with` 在生成完 SELECT 主体后,追加集合运算:
   ```rust
   for op in &state.set_operations {
       sql.push_str(match op.operator {
           SetOperator::Union => " UNION ",
           SetOperator::UnionAll => " UNION ALL ",
           SetOperator::Intersect => " INTERSECT ",
           SetOperator::Except => " EXCEPT ",
       });
       sql.push_str(&op.operand_sql);
   }
   ```
   参数合入 `all_params`。

4. QueryBuilder 添加方法:
   ```rust
   pub fn union(mut self, sql: impl Into<String>, params: Vec<DbValue>) -> Self { ... }
   pub fn union_all(mut self, sql: impl Into<String>, params: Vec<DbValue>) -> Self { ... }
   pub fn intersect(mut self, sql: impl Into<String>, params: Vec<DbValue>) -> Self { ... }
   pub fn except(mut self, sql: impl Into<String>, params: Vec<DbValue>) -> Self { ... }
   ```

**修改文件**:`crates/macros/src/linq.rs`

- `LinqClause` 添加 `Union { source: Expr }` / `UnionAll { source: Expr }` / `Intersect { source: Expr }` / `Except { source: Expr }`。
- 解析器:`union <subquery_expr>` —— subquery_expr 是一个返回 `(String, Vec<DbValue>)` 的表达式(或直接是另一个 `QueryBuilder`)。
- `expand_clauses` 调用对应方法。

**简化(D5)**:operand 不允许包含 ORDER BY/LIMIT(由调用方保证);operand 通过 `QueryBuilder::to_sql_with` 生成 SQL 字符串后传入。

### Part 2:递归 CTE

**目标**:支持 `linq!(ctx.set::<Employee>(); with recursive org_tree as |e: Employee| link e.manager_id to e.employee_id; from org_tree)` 语法。

**修改文件**:`crates/core/src/query.rs`

1. `CteSpec` 添加字段:
   ```rust
   pub(crate) is_recursive: bool,
   pub(crate) recursive_link: Option<(String, String)>,  // (fk_field, pk_field)
   ```

2. `to_sql_with` 中 CTE 生成逻辑:
   - 非递归:`WITH name AS (SELECT * FROM table WHERE <body>)`
   - 递归:`WITH RECURSIVE name AS (SELECT * FROM table WHERE <anchor_body> UNION ALL SELECT t.* FROM table t JOIN name ON t.<fk> = name.<pk>)`
   - 递归 CTE 的 anchor 由原 `where_expr` 提供(可选),递归成员的链接由 `recursive_link` 提供。

3. QueryBuilder 添加 `with_recursive_cte_typed` 方法:
   ```rust
   pub fn with_recursive_cte<E, F>(
       mut self,
       name: &str,
       link_fk: &str,
       link_pk: &str,
       filter: F,
   ) -> Self
   where
       F: FnOnce(...) -> BoolExpr,
   { ... }
   ```

**修改文件**:`crates/macros/src/linq.rs`

- `LinqClause::With` 添加可选 `recursive: bool` 和 `link: Option<(Expr, Expr)>` 字段。
- 解析器:在 `with` 后检测 `recursive` 关键字,然后检测 `link <fk> to <pk>` 语法。
- `expand_clauses` 调用 `with_recursive_cte_typed` 或 `with_cte_typed`。

**注意**:递归 CTE 的 anchor WHERE 由用户在 `with` 子句前的 `filter` 提供,或允许 `with recursive name as |e: T| <anchor_filter>; link e.fk to e.pk` 语法。采用后者,filter 作为 anchor,body 解析为 link 表达式。

### Part 4:CASE WHEN 表达式

**目标**:支持在 SELECT 投影和 WHERE 中使用 CASE WHEN。

**修改文件**:`crates/core/src/query.rs`

1. 新增标量表达式枚举(D7 仅支持标量列/字面量/CASE,不支持嵌套标量运算):
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

2. `BoolExpr` 添加 `Case` 变体(WHERE 上下文中的 CASE WHEN):
   ```rust
   Case {
       when_clauses: Vec<(BoolExpr, BoolExpr)>,  // (when, then)
       else_clause: Option<Box<BoolExpr>>,
   },
   ```
   **简化(D7)**:WHERE 上下文中 CASE 的 then/else 也是 BoolExpr(用于 `WHERE CASE WHEN ... THEN true ELSE false END`)。

3. `compile_bool_expr` 添加 `Case` 分支生成 SQL。

4. 新增 `compile_scalar_expr` 函数,生成标量表达式 SQL。

5. `SelectItem` 新增结构(若现 select 仅支持元组列):
   ```rust
   #[derive(Debug, Clone)]
   pub enum SelectItem {
       Column(String),
       Scalar(ScalarExpr),
       Aliased(ScalarExpr, String),
   }
   ```
   `QueryState::projected_columns` 改为 `Vec<SelectItem>` 或新增 `projected_scalars: Vec<SelectItem>`。

**修改文件**:`crates/macros/src/linq.rs`

- `LinqClause::Select` 扩展:支持 `case when <cond> then <val> [when ... else <val>] end` 表达式作为 select 项。
- 新增 `case when` 表达式解析器,生成 `ScalarExpr::Case`。
- `expand_clauses` 调用 `compile_scalar_expr` 生成 SQL 片段,放入 `projected_scalars`。

**简化(D7)**:CASE WHEN 仅支持 WHERE 布尔上下文和 SELECT 标量上下文,不支持 GROUP BY/HAVING/ORDER BY 中的 CASE(避免复杂度爆炸)。

### Part 5:UPSERT/MERGE

**目标**:支持 `ctx.set::<Blog>().upsert(blog)` API(D8:仅 API,不进 linq! 宏)。

**修改文件**:`crates/core/src/entity.rs`

- `EntityState` 添加 `Upsert` 变体:
  ```rust
  pub enum EntityState {
      Detached,
      Added,
      Unchanged,
      Modified,
      Deleted,
      Upsert,
  }
  ```

**修改文件**:`crates/core/src/db_set.rs`

- `DbSet<T>` 添加 `upsert(&mut self, entity: T)` 方法,设置 `EntityState::Upsert`。

**修改文件**:`crates/core/src/provider.rs`

- `ISqlGenerator` 添加 `upsert` 方法:
  ```rust
  fn upsert(
      &self,
      table: &str,
      insert_cols: &[&str],
      conflict_cols: &[&str],
      update_cols: &[&str],
  ) -> String;
  ```

**修改文件**:SQLite/PostgreSQL/MySQL provider 实现

- SQLite/PostgreSQL:`INSERT INTO ... (...) VALUES (...) ON CONFLICT (conflict_cols) DO UPDATE SET update_col = EXCLUDED.update_col, ...`
- MySQL:`INSERT INTO ... (...) VALUES (...) ON DUPLICATE KEY UPDATE update_col = VALUES(update_col), ...`

**修改文件**:`crates/core/src/change_executor.rs`

- 添加 `execute_upserts` 函数,与 `execute_inserts` 同构但使用 `gen.upsert(...)` 生成 SQL。
- EntityState::Upsert 实体进入 `execute_upserts` 分支。

**修改文件**:`crates/core/src/db_context.rs`

- `save_changes` 中区分 EntityState::Upsert,调用 `execute_upserts`。

### 全局:lib.rs prelude 导出 + CHANGELOG

**修改文件**:`crates/core/src/lib.rs`

- prelude 中导出:`ITransaction`, `DbTransaction`, `SetOperator`, `SetOpSpec`, `ScalarExpr`, `SelectItem`。

**修改文件**:`CHANGELOG.md`(若存在)

- 添加本次变更条目。

## 假设与决策

- **D1**:`commit` / `rollback` 消费 `self: Box<Self>`,使用后句柄不可再用。
- **D2**:Drop 不执行实际回滚(Rust async Drop 不可行);调用方需显式 commit/rollback。
- **D3**:`begin_transaction` 返回句柄但不注册 ambient;`use_transaction` 注册 ambient。分离手动控制与作用域控制。
- **D4**:接受 `Box::pin(async move)` 调用方样板(Rust async + 借用检查器的代价)。
- **D5**:集合运算的 operand 不允许包含 ORDER BY/LIMIT(简化)。
- **D6**:MySQL FULL OUTER JOIN 不模拟(已知限制,在 to_sql_with 中 panic 或返回错误)。
- **D7**:CASE WHEN 仅支持 WHERE 布尔上下文 + SELECT 标量上下文,不支持嵌套标量运算或 GROUP BY/HAVING/ORDER BY 中的 CASE。
- **D8**:UPSERT 仅 API,不进 linq! 宏(写操作 vs 查询宏的边界)。

## 验证步骤

每部分完成后运行对应验证:

1. **Part 0**:`cargo build -p rust-ef` + `cargo test -p rust-ef --test transaction_ext_tests` —— 现有测试改写后通过 + 4 个新 use_transaction 测试通过。
2. **Part 3**:`cargo build -p rust-ef --features macros` + `cargo test -p rust-ef --test linq_dsl_tests` 添加 3 个 JOIN 测试通过。
3. **Part 1**:`cargo test -p rust-ef --test linq_dsl_tests` 添加 3 个集合运算测试通过。
4. **Part 2**:`cargo test -p rust-ef --test cte_syntax_tests` 添加递归 CTE 测试通过。
5. **Part 4**:`cargo test -p rust-ef --test linq_dsl_tests` 添加 CASE WHEN 测试通过。
6. **Part 5**:`cargo test -p rust-ef --test sqlite_crud_tests` 添加 UPSERT 测试通过。
7. **全局**:`cargo build --workspace` + `cargo test --workspace` 全部通过。

## 实施顺序

1. **Part 0(ITransaction)** —— 重构 Priority 2,为后续提供稳定基础。
2. **Part 3(RIGHT/FULL/CROSS JOIN)** —— 最简单,JoinSpec 已就绪。
3. **Part 1(集合运算)** —— 独立模块,无依赖。
4. **Part 2(递归 CTE)** —— 基于 CteSpec 扩展。
5. **Part 4(CASE WHEN)** —— 需要新的标量 AST。
6. **Part 5(UPSERT)** —— 涉及 EntityState/Provider/ChangeExecutor 多处。
7. **全局验证 + CHANGELOG**。

## 任务清单

- [ ] Part 0: ITransaction 封装 —— 新建 transaction.rs + 重构 db_context.rs + 更新 transaction_ext_tests.rs
- [ ] Part 3: RIGHT/FULL/CROSS JOIN —— JoinSpec::to_sql 适配 + QueryBuilder 方法 + linq! 宏子句
- [ ] Part 1: 集合运算 UNION/INTERSECT/EXCEPT —— SetOperator + SetOpSpec + QueryState + linq! 宏子句
- [ ] Part 2: 递归 CTE —— CteSpec 递归字段 + with_recursive_cte_typed + linq! with recursive
- [ ] Part 4: CASE WHEN —— ScalarExpr + BoolExpr::Case + SelectItem + linq! case when
- [ ] Part 5: UPSERT —— EntityState::Upsert + DbSet::upsert + ISqlGenerator::upsert + execute_upserts
- [ ] 全局验证 + CHANGELOG + lib.rs prelude 导出
