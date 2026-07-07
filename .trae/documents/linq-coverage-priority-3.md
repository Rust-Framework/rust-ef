# Priority 3: linq 覆盖度补齐 + ITransaction 封装

## Summary

本轮迭代完成两大块工作:

1. **ITransaction 封装**(Priority 2 重构)— 把 Priority 2 散落在 `DbContext` 上的事务方法(`begin_transaction`/`commit_transaction`/`create_savepoint`/...)抽象为 `ITransaction` trait,提供统一的事务句柄;并新增 `DbContext::use_transaction(f)` 作用域执行 API。
2. **Priority 3 全部 5 个 linq 子项**:
   - 3a 集合运算(UNION / UNION ALL / INTERSECT / EXCEPT)
   - 3b 递归 CTE(WITH RECURSIVE)
   - 3c 额外 JOIN 类型(RIGHT / FULL OUTER / CROSS)
   - 3d CASE WHEN(标量表达式 AST 扩展)
   - 3e UPSERT / MERGE(ON CONFLICT / ON DUPLICATE KEY / INSERT OR REPLACE)

## Current State Analysis

### 事务接口(Priority 2 现状)

- `DbContext` 字段 `ambient_transaction: Option<Box<dyn IAsyncConnection>>` 直接持有原始连接
- `begin_transaction(&mut self) -> EFResult<()>` / `commit_transaction` / `rollback_transaction` / `create_savepoint` / `release_savepoint` / `rollback_to_savepoint` / `set_transaction_isolation` 全部是 `DbContext` 上的具体方法
- `save_changes()` 用 `TxnSource` 枚举(Ambient / Managed)区分环境事务与自管事务
- `use_transaction_scope` 因 Rust async 借用检查器限制被放弃
- `IAsyncConnection` trait 已包含 `create_savepoint` / `release_savepoint` / `rollback_to_savepoint` / `set_transaction_isolation` 4 个方法

### linq! 宏现状(已确认)

- **三种形式**:A(过滤闭包)、B(多子句查询)、C(值产生:filter/index/key)
- **已有子句**:`include`/`order_by`/`group_by`/`select`/`having`/`sum`/`avg`/`min`/`max`/`count`/`distinct`/`set`+`execute_update`/`take`/`skip`/`window`/`with`(typed CTE)/`from`/`inner_join`/`left_join`
- **过滤方法**:6 种比较 + `&&`/`||`/`!` + `is_null`/`is_not_null`/`contains`(IN/LIKE)/`starts_with`/`ends_with`/`between`/`any`/`none`/`all`(EXISTS 子查询)/`in_subquery`(IN SELECT)

### 确认的缺口

| 缺口 | 位置 | 现状 |
|------|------|------|
| 集合运算 | `QueryState` | 无 `set_operations` 字段;`to_sql_with` 不生成 UNION/INTERSECT/EXCEPT |
| 递归 CTE | `CteSpec` | 无 `is_recursive` 标志;typed 模式只支持单条 `SELECT * FROM T WHERE expr`;SQL gen 只发 `WITH` 不发 `WITH RECURSIVE` |
| RIGHT/FULL/CROSS JOIN | `linq!` 宏 | `JoinSpec.join_type` 是 String(可填任意值),但宏只暴露 `inner_join`/`left_join` |
| CASE WHEN | `BoolExpr` | 纯布尔 AST(无标量分支);`FilterCondition` 只支持 `col op value` |
| UPSERT/MERGE | `DbSet`/`ChangeExecutor`/`ISqlGenerator` | 完全缺失 |

### 关键文件

- `crates/core/src/provider.rs` — `IAsyncConnection` trait、`IsolationLevel`、`ISqlGenerator` trait
- `crates/core/src/query.rs` — `BoolExpr`、`QueryState`、`QueryBuilder`、`CteSpec`、`JoinSpec`、`to_sql_with`
- `crates/core/src/db_context.rs` — `DbContext`、ambient transaction 字段与方法
- `crates/core/src/change_executor.rs` — `execute_inserts`/`execute_updates`/`execute_deletes`、`generate_insert_sql`/`generate_update_sql`/`generate_delete_sql`
- `crates/core/src/entity.rs` — `EntityState` 枚举(Detached/Added/Unchanged/Modified/Deleted)
- `crates/core/src/db_set.rs` — `DbSet` API
- `crates/macros/src/linq.rs` — `linq!` 宏(LinqClause 枚举、parse_*、expand_*)
- `crates/sqlite/src/connection.rs`、`crates/postgres/src/connection.rs`、`crates/mysql/src/connection.rs` — `IAsyncConnection` 实现
- `crates/sqlite/src/sql_generator.rs`、`crates/postgres/src/sql_generator.rs`、`crates/mysql/src/sql_generator.rs` — `ISqlGenerator` 实现

---

## Part 0: ITransaction 封装

### What

把 `DbContext` 上的事务方法抽象为 `ITransaction` trait,提供统一的事务句柄。`DbContext::begin_transaction()` 返回 `Box<dyn ITransaction>`,调用方通过句柄操作事务。新增 `DbContext::use_transaction(f)` 作用域执行 API。

### Why

- Priority 2 把事务状态(`ambient_transaction`)和事务方法散落在 `DbContext` 上,职责不清
- `ITransaction` 句柄抽象符合 EFCore 的 `IDbContextTransaction` 设计,便于测试 mock 与 provider 自定义实现
- `use_transaction(f)` 提供 RAII 风格的作用域事务,自动 commit/rollback
- 为后续扩展(分布式事务、嵌套事务)留出 trait 接口

### How

#### 0.1 新增 `ITransaction` trait(`crates/core/src/provider.rs` 或新建 `crates/core/src/transaction.rs`)

```rust
/// 事务句柄,封装环境事务的生命周期。
///
/// 由 `DbContext::begin_transaction()` 返回。调用方通过此句柄操作事务;
/// `DbContext::save_changes()` 在事务活跃时自动复用。
///
/// 未 commit 即 drop 时,实现应回滚事务(RAII 语义)。
#[async_trait]
pub trait ITransaction: Send {
    /// 提交事务。消费句柄,后续 save_changes 不再复用此事务。
    async fn commit(self: Box<Self>) -> EFResult<()>;
    /// 回滚事务。消费句柄。
    async fn rollback(self: Box<Self>) -> EFResult<()>;
    /// 在当前事务内创建保存点。
    async fn create_point(&mut self, name: &str) -> EFResult<()>;
    /// 释放(提交)保存点,丢弃其回滚点。
    async fn release_point(&mut self, name: &str) -> EFResult<()>;
    /// 回滚到命名保存点,保留外层事务。
    async fn rollback_point(&mut self, name: &str) -> EFResult<()>;
    /// 设置隔离级别。必须在 begin 之后、任何查询之前调用。
    async fn set_isolation(&mut self, level: IsolationLevel) -> EFResult<()>;
}
```

**决策 D1**:`commit`/`rollback` 消费 `self: Box<Self>`,而非 `&mut self` — 保证句柄用后即弃,避免双重提交。RAII Drop 作为未显式 commit 的兜底回滚。

#### 0.2 通用实现 `DbTransaction`(`crates/core/src/transaction.rs`)

```rust
/// 通用事务句柄,包装 `IAsyncConnection` 并委托所有操作。
pub struct DbTransaction {
    conn: Box<dyn IAsyncConnection>,
    committed: bool,
}

impl DbTransaction {
    pub fn new(conn: Box<dyn IAsyncConnection>) -> Self {
        Self { conn, committed: false }
    }
}

#[async_trait]
impl ITransaction for DbTransaction {
    async fn commit(mut self: Box<Self>) -> EFResult<()> {
        self.conn.commit_transaction().await?;
        self.committed = true;
        Ok(())
    }
    async fn rollback(mut self: Box<Self>) -> EFResult<()> {
        self.conn.rollback_transaction().await?;
        self.committed = true; // 已显式结束
        Ok(())
    }
    async fn create_point(&mut self, name: &str) -> EFResult<()> {
        self.conn.create_savepoint(name).await
    }
    async fn release_point(&mut self, name: &str) -> EFResult<()> {
        self.conn.release_savepoint(name).await
    }
    async fn rollback_point(&mut self, name: &str) -> EFResult<()> {
        self.conn.rollback_to_savepoint(name).await
    }
    async fn set_isolation(&mut self, level: IsolationLevel) -> EFResult<()> {
        self.conn.set_transaction_isolation(level).await
    }
}

impl Drop for DbTransaction {
    fn drop(&mut self) {
        if !self.committed {
            // RAII 兜底回滚 — 同步执行(best-effort)。
            // async runtime 不可在 Drop 中使用,因此跳过;
            // 调用方应显式 commit/rollback。
            // 此处仅标记,不实际回滚(async 限制)。
        }
    }
}
```

**决策 D2**:不在 `Drop` 中实际回滚(async Drop 不可行)。文档明确要求调用方显式 `commit`/`rollback` 或使用 `use_transaction`。`use_transaction` 通过 `match` 结果自动调用。

#### 0.3 `DbContext` 重构(`crates/core/src/db_context.rs`)

**字段变更**:
```rust
// Before (Priority 2):
ambient_transaction: Option<Box<dyn IAsyncConnection>>,

// After:
ambient_transaction: Option<Box<dyn ITransaction>>,
```

**方法变更**(全部破坏性):
```rust
// Before:
pub async fn begin_transaction(&mut self) -> EFResult<()>
pub async fn commit_transaction(&mut self) -> EFResult<()>
pub async fn rollback_transaction(&mut self) -> EFResult<()>
pub async fn create_savepoint(&mut self, name: &str) -> EFResult<()>
pub async fn release_savepoint(&mut self, name: &str) -> EFResult<()>
pub async fn rollback_to_savepoint(&mut self, name: &str) -> EFResult<()>
pub async fn set_transaction_isolation(&mut self, level: IsolationLevel) -> EFResult<()>

// After:
pub async fn begin_transaction(&mut self) -> EFResult<Box<dyn ITransaction>>
// commit/rollback/savepoint/isolation 全部移除 — 由 ITransaction 句柄提供
```

**`begin_transaction` 实现**:
```rust
pub async fn begin_transaction(&mut self) -> EFResult<Box<dyn ITransaction>> {
    if self.ambient_transaction.is_some() {
        return Err(EFError::Transaction(
            "ambient transaction already active; commit or rollback first".into(),
        ));
    }
    let mut conn = self.provider.get_connection().await?;
    conn.begin_transaction().await?;
    let txn: Box<dyn ITransaction> = Box::new(DbTransaction::new(conn));
    // 注意:不存入 ambient_transaction — 由调用方持有句柄。
    // 但 save_changes 需要复用此事务,因此仍需 ambient 注册。
    Ok(txn)
}
```

**决策 D3**:`begin_transaction` 返回句柄**并**注册到 `ambient_transaction`。调用方持有句柄用于 commit/rollback/savepoint,`DbContext` 持有 ambient 引用用于 `save_changes` 复用。由于 `Box<dyn ITransaction>` 不能 Clone,改为 `Arc<Mutex<...>>`? 

**修正 D3**:保持 Priority 2 的 `take()`/restore 模式。`begin_transaction` 把事务句柄存入 `ambient_transaction`,返回一个轻量 `TransactionHandle` 引用(持有 `&mut DbContext` 或内部用 `Arc`):

实际上最简洁的方案:**`begin_transaction` 把事务存入 ambient,返回 `&mut self.ambient_transaction`**。但生命周期复杂。

**最终决策 D3**(简化):`begin_transaction` 返回 `Box<dyn ITransaction>`,**不**自动注册 ambient。调用方若要让 `save_changes` 复用,改用 `use_transaction(f)` 作用域 API(见 0.4)。手动 `begin` 返回的句柄仅供手动操作,`save_changes` 仍走自管事务路径。

这比 Priority 2 更清晰:手动 begin = 手动控制(句柄自持);`use_transaction` = 作用域控制(ambient 自动复用)。

#### 0.4 `DbContext::use_transaction`(`crates/core/src/db_context.rs`)

```rust
impl DbContext {
    /// 作用域事务:begin → f(ctx) → commit/rollback。
    ///
    /// 在 `f` 执行期间,`save_changes()` 复用环境事务。
    /// `f` 返回 Ok → commit;返回 Err → rollback。
    pub async fn use_transaction<'a, F, R>(&'a mut self, f: F) -> EFResult<R>
    where
        F: FnOnce(&'a mut Self) -> Pin<Box<dyn Future<Output = EFResult<R>> + Send + 'a>>,
    {
        // begin
        let txn_conn = {
            let mut conn = self.provider.get_connection().await?;
            conn.begin_transaction().await?;
            conn
        };
        self.ambient_transaction = Some(Box::new(DbTransaction::new(txn_conn)));

        // run
        let result = f(self).await;

        // end: take 事务,根据结果 commit/rollback
        let txn = self.ambient_transaction.take().expect("ambient vanished");
        match result {
            Ok(r) => {
                txn.commit().await?;
                Ok(r)
            }
            Err(e) => {
                let _ = txn.rollback().await;
                Err(e)
            }
        }
    }
}
```

**关键**:用 `Pin<Box<dyn Future + Send + 'a>>` 绕过借用检查器。`f(self)` 借用 `&'a mut Self`,返回的 Future 生命周期绑定到 `'a`;`.await` 完成后借用释放,`self.ambient_transaction.take()` 可以执行。

调用方用法:
```rust
ctx.use_transaction(|ctx| Box::pin(async move {
    ctx.set::<Blog>().add(blog);
    ctx.save_changes().await?;
    Ok(())
})).await?;
```

**决策 D4**:接受 `Box::pin(async move)` 的调用方样板代码。这是 Rust async + 借用检查器的必要代价。提供 `#[rust_ef::transaction]` 属性宏作为未来增强(非本轮范围)。

#### 0.5 `save_changes()` 适配

`save_changes()` 的 `TxnSource` 枚举改为:
```rust
enum TxnSource {
    Ambient(Box<dyn ITransaction>),  // 从 ITransaction 获取底层连接
    Managed(Box<dyn IAsyncConnection>),
}
```

但 `ITransaction` 不暴露 `&mut IAsyncConnection`!需要在 `ITransaction` 上加一个方法:
```rust
#[async_trait]
pub trait ITransaction: Send {
    // ... 上述方法 ...
    /// 获取底层连接引用,供 save_changes 执行 INSERT/UPDATE/DELETE。
    /// 实现应返回 `&mut dyn IAsyncConnection`。
    fn connection(&mut self) -> &mut (dyn IAsyncConnection + Send);
}
```

`DbTransaction::connection(&mut self) -> &mut dyn IAsyncConnection` 返回 `&mut *self.conn`。

`save_changes()` 中:
```rust
let mut txn = match self.ambient_transaction.take() {
    Some(t) => TxnSource::Ambient(t),
    None => { /* begin, TxnSource::Managed */ }
};
let conn_ref: &mut (dyn IAsyncConnection + Send) = match &mut txn {
    TxnSource::Ambient(t) => t.connection(),
    TxnSource::Managed(c) => &mut **c,
};
// 执行 INSERT/UPDATE/DELETE...
match result {
    Ok(_) => { if let TxnSource::Managed(c) = txn { c.commit_transaction().await?; } else { /* Ambient: 恢复 */ self.ambient_transaction = Some(t); } }
    Err(e) => { /* Managed: rollback; Ambient: 恢复 */ }
}
```

#### 0.6 文档更新

- `CHANGELOG.md` 新增 `### Changed — ITransaction abstraction (priority 2 refactor)` 段落
- `crates/core/src/lib.rs` prelude 导出 `ITransaction`
- `crates/core/src/db_context.rs` 模块文档更新事务用法示例

### Verification

- `cargo build -p rust-ef -p rust-ef-sqlite` 编译通过
- `cargo test -p rust-ef --test transaction_ext_tests` 现有 12 个测试适配新 API 后全通过
- 新增 `transaction_scope_tests.rs`:4 个测试覆盖 `use_transaction` 的 Ok commit / Err rollback / 嵌套 savepoint / 隔离级别

---

## Part 1: 3a 集合运算(UNION / UNION ALL / INTERSECT / EXCEPT)

### What

`linq!` 宏新增 `union`/`union_all`/`intersect`/`except` 子句,每个子句接收一个 Form A 风格闭包 `|b: T| ...` 构建 RHS 查询。`QueryState` 新增 `set_operations: Vec<SetOpSpec>` 字段。SQL gen 在主 SELECT 后追加 `{op} {operand_sql}`。

### Why

集合运算是 SQL 标准的核心组合原语,用于合并同构结果集(如 `已发布博客 UNION 草稿博客`)。现有 CTE + 子查询可间接实现,但语法笨重。原生 UNION 支持让 `linq!` 覆盖剩余 20% OLTP 场景。

### How

#### 1.1 新增类型(`crates/core/src/query.rs`)

```rust
/// 集合运算符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOperator {
    Union,
    UnionAll,
    Intersect,
    Except,
}

impl SetOperator {
    pub fn as_sql(&self) -> &'static str {
        match self {
            SetOperator::Union => "UNION",
            SetOperator::UnionAll => "UNION ALL",
            SetOperator::Intersect => "INTERSECT",
            SetOperator::Except => "EXCEPT",
        }
    }
}

/// 集合运算规格:主 SELECT 与 operand 查询之间的运算。
#[derive(Debug, Clone)]
pub struct SetOpSpec {
    pub operator: SetOperator,
    /// RHS 查询的完整 QueryState(独立 SELECT/FROM/WHERE/...)。
    pub operand: Box<QueryState>,
}
```

#### 1.2 `QueryState` 新增字段

```rust
pub struct QueryState {
    // ... 现有字段 ...
    /// 集合运算链:按顺序追加到主 SELECT 之后。
    pub set_operations: Vec<SetOpSpec>,
}
```

`QueryState::new` 初始化 `set_operations: Vec::new()`。

#### 1.3 `to_sql_with` 追加集合运算生成

在现有 SQL 构建末尾(LIMIT/OFFSET 之前,CTE 之后):
```rust
// 集合运算 — 在主 SELECT 后追加 {op} {operand_sql}
for set_op in &self.set_operations {
    let operand_sql = set_op.operand.to_sql_with(gen);
    sql.push_str(&format!(" {} {}", set_op.operator.as_sql(), operand_sql));
}
```

**注意**:集合运算的参数顺序 — operand 的参数追加在主查询参数之后。`all_params()` 需扩展:
```rust
pub fn all_params(&self) -> Vec<DbValue> {
    let mut params = Vec::new();
    for cte in &self.ctes { params.extend(cte.params.clone()); }
    params.extend(self.parameters.clone());
    for set_op in &self.set_operations {
        params.extend(set_op.operand.all_params());
    }
    params
}
```

#### 1.4 `QueryBuilder` 新增方法

```rust
impl<T> QueryBuilder<T> {
    /// `#[doc(hidden)]` — `linq!(union |b: T| ...)` 展开。
    #[doc(hidden)]
    pub fn union_internal<F>(mut self, f: F) -> Self
    where
        F: FnOnce(QueryBuilder<T>) -> QueryBuilder<T>,
    {
        let rhs = f(QueryBuilder::new_with_provider(
            self.provider.clone(),
            self.state.from.clone(),
        ));
        self.state.set_operations.push(SetOpSpec {
            operator: SetOperator::Union,
            operand: Box::new(rhs.state),
        });
        self
    }
    // union_all_internal / intersect_internal / except_internal 类似
}
```

**前提**:`QueryBuilder` 需要 `new_with_provider(provider, from)` 构造器(若不存在则新增),以及 `provider` 字段可 `clone`(`Arc<dyn IDatabaseProvider>` 已满足)。

#### 1.5 宏子句(`crates/macros/src/linq.rs`)

`LinqClause` 枚举新增:
```rust
/// `union |b: T| <where_body>` — RHS 查询的过滤闭包(Form A 风格)。
Union { entity: Type, param: Ident, body: Expr },
UnionAll { entity: Type, param: Ident, body: Expr },
Intersect { entity: Type, param: Ident, body: Expr },
Except { entity: Type, param: Ident, body: Expr },
```

`LinqClause::parse` 新增 4 个分支:
```rust
"union" => parse_set_op_rest(input, SetOpKind::Union),
"union_all" => parse_set_op_rest(input, SetOpKind::UnionAll),
"intersect" => parse_set_op_rest(input, SetOpKind::Intersect),
"except" => parse_set_op_rest(input, SetOpKind::Except),
```

`parse_set_op_rest` 解析 `|b: T| body`(复用 `parse_typed_closure`):
```rust
enum SetOpKind { Union, UnionAll, Intersect, Except }

fn parse_set_op_rest(input: ParseStream, kind: SetOpKind) -> Result<LinqClause> {
    let (entity, param, body) = parse_typed_closure(input)?;
    Ok(match kind {
        SetOpKind::Union => LinqClause::Union { entity, param, body },
        SetOpKind::UnionAll => LinqClause::UnionAll { entity, param, body },
        SetOpKind::Intersect => LinqClause::Intersect { entity, param, body },
        SetOpKind::Except => LinqClause::Except { entity, param, body },
    })
}
```

`expand_clauses` 新增 4 个分支:
```rust
LinqClause::Union { entity, param, body } => {
    let ctx = LinqCtx::single(entity, Some(param));
    let where_chain = compile_expr(&ctx, body)?;
    let method = Ident::new("union_internal", Span::call_site());
    chain = quote! { #chain .#method(|__qb: rust_ef::query::QueryBuilder<#entity>| { __qb #where_chain }) };
}
// UnionAll/Intersect/Except 类似,method 名分别为 union_all_internal/intersect_internal/except_internal
```

#### 1.6 SQL gen 顺序确认

集合运算在 SQL 中的位置:`WITH cte... SELECT ... FROM ... WHERE ... UNION SELECT ... LIMIT ...`
- CTE 前缀在最前
- 主 SELECT + WHERE + GROUP BY + HAVING
- UNION operand SQL(operand 自身是完整 SELECT,不含 CTE/LIMIT)
- ORDER BY / LIMIT / OFFSET 在最后(应用于合并结果)

**决策 D5**:operand 查询的 `to_sql_with` 会生成完整 SQL(含 ORDER BY/LIMIT)。SQL 标准允许在 UNION 的最后一个 SELECT 上加 ORDER BY/LIMIT。为简化,operand 的 ORDER BY/LIMIT 原样输出;主查询的 ORDER BY/LIMIT 在所有集合运算之后。若 operand 有 ORDER BY/LIMIT,需用括号包裹 — 本轮暂不包裹,文档说明 operand 不应含 ORDER BY/LIMIT。

### Verification

- 新增 `set_op_tests.rs`:6 个测试(UNION / UNION ALL / INTERSECT / EXCEPT 各 1 + 组合 + 参数顺序)
- `cargo test -p rust-ef --test set_op_tests` 全通过

---

## Part 2: 3b 递归 CTE(WITH RECURSIVE)

### What

`linq!` 宏新增 `with recursive <name> as |e: T| <anchor_where> link e.<fk> to e.<pk>` 子句,生成 `WITH RECURSIVE name AS (SELECT * FROM T WHERE <anchor> UNION ALL SELECT T.* FROM T JOIN name ON T.fk = name.id)`。`CteSpec` 新增 `is_recursive` 与 `recursive_link` 字段。

### Why

层级数据(组织架构、分类树、评论线程)是 OLTP 常见场景。递归 CTE 是 SQL 标准的层级查询原语,无法用普通 JOIN 替代。现有 typed CTE 只支持单条 SELECT,无法表达自引用递归。

### How

#### 2.1 `CteSpec` 扩展(`crates/core/src/query.rs`)

```rust
#[non_exhaustive]
pub struct CteSpec {
    // ... 现有字段 ...
    /// 是否为递归 CTE。true 时 SQL gen 发 `WITH RECURSIVE`。
    pub is_recursive: bool,
    /// 递归 CTE 的自连接条件:(child_fk_column, parent_pk_column)。
    /// 仅当 is_recursive=true 时有意义。生成 `JOIN name ON T.fk = name.pk`。
    pub recursive_link: Option<(String, String)>,
}
```

`with_cte_internal` / `with_cte_typed` 初始化新字段为 `false` / `None`。

#### 2.2 `QueryBuilder::with_recursive_cte_typed`

```rust
/// `#[doc(hidden)]` — `linq!(with recursive <name> as |e: T| <anchor> link e.fk to e.pk)` 展开。
#[doc(hidden)]
pub fn with_recursive_cte_typed(
    mut self,
    name: &str,
    table: &str,
    anchor_where: BoolExpr,
    link_fk: &'static str,
    link_pk: &'static str,
) -> Self {
    let params = collect_bool_expr_values(&anchor_where);
    let cte = CteSpec {
        name: name.to_string(),
        sql: String::new(),
        table: table.to_string(),
        where_expr: Some(anchor_where),
        params,
        columns: Vec::new(),
        is_recursive: true,
        recursive_link: Some((link_fk.to_string(), link_pk.to_string())),
    };
    self.state.ctes.push(cte);
    self
}
```

#### 2.3 `to_sql_with` 递归 CTE 生成

在 CTE 前缀生成段(L947-988),支持递归:
```rust
if !self.ctes.is_empty() {
    let any_recursive = self.ctes.iter().any(|c| c.is_recursive);
    let with_kw = if any_recursive { "WITH RECURSIVE" } else { "WITH" };
    let mut running_idx = 1usize;
    let mut cte_parts: Vec<String> = Vec::with_capacity(self.ctes.len());
    for c in &self.ctes {
        let body = if c.is_recursive {
            // 递归模式:anchor + UNION ALL + recursive
            let mut cte_idx = running_idx;
            let table = gen.quote_identifier(&c.table);
            let anchor_where = match &c.where_expr {
                Some(expr) => {
                    let where_sql = compile_bool_expr(expr, gen, &mut cte_idx);
                    format!("SELECT * FROM {} WHERE {}", table, where_sql)
                }
                None => format!("SELECT * FROM {}", table),
            };
            running_idx = cte_idx;
            let (fk, pk) = c.recursive_link.as_ref()
                .expect("recursive CTE must have recursive_link");
            let recursive = format!(
                "SELECT t.* FROM {} t JOIN {} ON t.{} = {}.{}",
                table, c.name, fk, c.name, pk
            );
            format!("{} UNION ALL {}", anchor_where, recursive)
        } else if !c.table.is_empty() {
            // 现有 typed 模式 — 不变
        } else {
            // 现有 raw 模式 — 不变
        };
        // ... columns 处理不变 ...
    }
    sql = format!("{} {} {}", with_kw, cte_parts.join(", "), sql);
}
```

#### 2.4 宏子句(`crates/macros/src/linq.rs`)

`LinqClause` 新增:
```rust
/// `with recursive <name> as |e: T| <anchor> link e.<fk> to e.<pk>`
WithRecursive {
    name: String,
    entity: Type,
    param: Ident,
    body: Expr,
    fk: String,  // 解析自 `e.<fk>`
    pk: String,  // 解析自 `e.<pk>`
},
```

`LinqClause::parse` 中 `"with"` 分支后检查 `recursive`:
```rust
"with" => {
    let cursor = input.cursor();
    if let Some((ident, _)) = cursor.ident() {
        if ident == "recursive" {
            return parse_with_recursive_rest(input);
        }
    }
    parse_with_rest(input)
}
```

`parse_with_recursive_rest`:
```rust
fn parse_with_recursive_rest(input: ParseStream) -> Result<LinqClause> {
    let _recursive: Ident = input.parse()?;  // consume "recursive"
    let name: Ident = input.parse()?;
    let _: Token![as] = input.parse()?;
    let (entity, param, body) = parse_typed_closure(input)?;
    // 解析 `link e.fk to e.pk`
    let link_kw: Ident = input.parse()?;
    debug_assert_eq!(link_kw, "link");
    let fk_expr: Expr = input.parse()?;  // e.parent_id
    let to_kw: Ident = input.parse()?;
    debug_assert_eq!(to_kw, "to");
    let pk_expr: Expr = input.parse()?;  // e.id
    let fk = extract_field_name_only(&fk_expr)?;
    let pk = extract_field_name_only(&pk_expr)?;
    Ok(LinqClause::WithRecursive { name: name.to_string(), entity, param, body, fk, pk })
}
```

`expand_clauses`:
```rust
LinqClause::WithRecursive { name, entity, param, body, fk, pk } => {
    let cte_ctx = LinqCtx::single(entity, Some(param));
    let bool_expr_code = compile_bool_expr(&cte_ctx, body)?;
    let name_str = name.as_str();
    let fk_const = field_const(entity, &fk, FieldKind::Column);
    let pk_const = field_const(entity, &pk, FieldKind::Column);
    chain = quote! {
        #chain .with_recursive_cte_typed(
            #name_str,
            <#entity>::TABLE,
            #bool_expr_code,
            #fk_const,
            #pk_const,
        )
    };
}
```

### Verification

- 新增 `recursive_cte_tests.rs`:3 个测试
  1. 分类树:parent_id 自引用,查询某节点的所有子孙
  2. anchor 无 WHERE(`SELECT * FROM T`)+ 递归
  3. 多级深度(3+ 层)验证
- `cargo test -p rust-ef --test recursive_cte_tests` 全通过

---

## Part 3: 3c 额外 JOIN 类型(RIGHT / FULL OUTER / CROSS)

### What

`linq!` 宏新增 `right_join`/`full_join`/`cross_join` 子句。`QueryBuilder` 新增 `right_join_internal`/`full_join_internal`/`cross_join_internal`。SQL gen 复用现有 `JoinSpec`(join_type 是 String)。

### Why

RIGHT JOIN 用于多对一反向查询;FULL OUTER JOIN 用于对账(找出两表不匹配行);CROSS JOIN 用于笛卡尔积(如生成所有组合)。SQL 标准基本操作,`JoinSpec` 已支持,只需暴露宏入口。

### How

#### 3.1 `QueryBuilder` 新增方法(`crates/core/src/query.rs`)

```rust
#[doc(hidden)]
pub fn right_join_internal(mut self, table: &'static str, left_column: &'static str, right_column: &'static str) -> Self {
    let on_clause = format!("{}.{} = {}.{}", self.state.from, left_column, table, right_column);
    self.state.joins.push(JoinSpec {
        join_type: "RIGHT".to_string(),
        table: table.to_string(),
        on_clause,
    });
    self
}

#[doc(hidden)]
pub fn full_join_internal(...) -> Self {
    // join_type: "FULL OUTER"
}

#[doc(hidden)]
pub fn cross_join_internal(mut self, table: &'static str) -> Self {
    // CROSS JOIN 无 ON 条件
    self.state.joins.push(JoinSpec {
        join_type: "CROSS".to_string(),
        table: table.to_string(),
        on_clause: String::new(),  // 空 on_clause
    });
    self
}
```

#### 3.2 `JoinSpec::to_sql` 适配 CROSS JOIN

```rust
impl JoinSpec {
    pub fn to_sql(&self) -> String {
        if self.on_clause.is_empty() {
            // CROSS JOIN — 无 ON
            format!("{} JOIN {}", self.join_type, self.table)
        } else {
            format!("{} JOIN {} ON {}", self.join_type, self.table, self.on_clause)
        }
    }
}
```

#### 3.3 宏子句(`crates/macros/src/linq.rs`)

`LinqClause` 新增:
```rust
RightJoin { params: Vec<(Ident, Type)>, left: Expr, right: Expr },
FullJoin { params: Vec<(Ident, Type)>, left: Expr, right: Expr },
CrossJoin { entity: Type },  // 无 ON 条件,只需表名
```

`LinqClause::parse`:
```rust
"right_join" => parse_join_rest(input, JoinKind::Right),
"full_join" => parse_join_rest(input, JoinKind::Full),
"cross_join" => parse_cross_join_rest(input),
```

`parse_join_rest` 现有(支持 inner/left)扩展为 `JoinKind` 枚举:
```rust
enum JoinKind { Inner, Left, Right, Full }
```

`parse_cross_join_rest`:
```rust
fn parse_cross_join_rest(input: ParseStream) -> Result<LinqClause> {
    // cross_join T2 — 只需实体类型
    let entity: Type = input.parse()?;
    Ok(LinqClause::CrossJoin { entity })
}
```

`expand_clauses` 新增分支,复用 `expand_join`(Right/Full),CrossJoin 单独处理:
```rust
LinqClause::CrossJoin { entity } => {
    let table = quote! { <#entity>::TABLE };
    chain = quote! { #chain .cross_join_internal(#table) };
}
```

#### 3.4 方言兼容性

| JOIN 类型 | SQLite | PostgreSQL | MySQL |
|-----------|--------|------------|-------|
| INNER | ✅ | ✅ | ✅ |
| LEFT | ✅ | ✅ | ✅ |
| RIGHT | ✅ (3.39+) | ✅ | ✅ |
| FULL OUTER | ✅ (3.39+) | ✅ | ❌(需 UNION 模拟) |
| CROSS | ✅ | ✅ | ✅ |

**决策 D6**:本轮不实现 MySQL FULL OUTER JOIN 模拟。`full_join` 在 MySQL 上运行时报 SQL 错误(由数据库返回),文档说明限制。未来可在 `ISqlGenerator` 加 `supports_full_outer_join()` 方法并生成 UNION 模拟 SQL。

### Verification

- `linq_dsl_tests.rs` 新增 3 个测试:`test_right_join`/`test_full_join`/`test_cross_join`
- `cargo test -p rust-ef --test linq_dsl_tests` 全通过

---

## Part 4: 3d CASE WHEN(标量表达式 AST 扩展)

### What

`BoolExpr` 新增 `Case` 变体(WHERE/HAVING 中的 CASE WHEN)。新增 `ScalarExpr` 枚举(SELECT 投影与 SET 子句中的标量表达式)。`linq!` 宏新增 `case when <cond> then <expr> else <expr> end` 语法。

### Why

CASE WHEN 是 SQL 条件逻辑原语,用于:
- WHERE 中:`WHERE CASE WHEN x THEN 1 ELSE 0 END = 1`
- SELECT 中:`SELECT CASE WHEN x THEN 'A' ELSE 'B' END AS grade`
- SET 中:`UPDATE t SET grade = CASE WHEN score > 90 THEN 'A' ELSE 'B' END`

现有 `BoolExpr` 是纯布尔 AST,无法表达标量条件逻辑。

### How

#### 4.1 新增 `ScalarExpr`(`crates/core/src/query.rs`)

```rust
/// 标量表达式 AST — 用于 SELECT 投影与 SET 子句。
#[derive(Debug, Clone)]
pub enum ScalarExpr {
    /// 列引用:`t.col` 或 `col`。
    Column(String),
    /// 字面值。
    Literal(DbValue),
    /// CASE WHEN ... THEN ... ELSE ... END
    Case {
        when_clauses: Vec<(BoolExpr, ScalarExpr)>,  // (condition, result)
        else_clause: Option<Box<ScalarExpr>>,
    },
    /// 聚合函数:`SUM(col)`、`COUNT(*)` 等。
    Aggregate { func: String, col: Option<String> },
    /// 二元运算:`a + b`、`a * 2` 等。
    Binary { op: String, left: Box<ScalarExpr>, right: Box<ScalarExpr> },
}

impl ScalarExpr {
    pub fn to_sql(&self, gen: &dyn ISqlGenerator, param_idx: &mut usize) -> String {
        match self {
            ScalarExpr::Column(c) => gen.quote_identifier(c),
            ScalarExpr::Literal(v) => {
                *param_idx += 1;
                gen.parameter_placeholder(*param_idx).to_string()
            }
            ScalarExpr::Case { when_clauses, else_clause } => {
                let parts: Vec<String> = when_clauses.iter()
                    .map(|(cond, result)| {
                        format!("WHEN {} THEN {}", 
                            compile_bool_expr(cond, gen, param_idx),
                            result.to_sql(gen, param_idx))
                    })
                    .collect();
                let else_part = else_clause.as_ref()
                    .map(|e| format!(" ELSE {}", e.to_sql(gen, param_idx)))
                    .unwrap_or_default();
                format!("CASE {}{} END", parts.join(" "), else_part)
            }
            ScalarExpr::Aggregate { func, col } => {
                match col {
                    Some(c) => format!("{}({})", func, gen.quote_identifier(c)),
                    None => format!("{}(*)", func),
                }
            }
            ScalarExpr::Binary { op, left, right } => {
                format!("{} {} {}", left.to_sql(gen, param_idx), op, right.to_sql(gen, param_idx))
            }
        }
    }
}
```

#### 4.2 `BoolExpr` 新增 `Case` 变体(布尔上下文 CASE)

```rust
pub enum BoolExpr {
    // ... 现有变体 ...
    /// CASE WHEN ... THEN ... ELSE ... END 在布尔上下文。
    /// SQL: `CASE WHEN cond1 THEN bool1 WHEN cond2 THEN bool2 ELSE bool3 END`
    Case {
        when_clauses: Vec<(BoolExpr, BoolExpr)>,
        else_clause: Option<Box<BoolExpr>>,
    },
}
```

`compile_bool_expr` 新增 `BoolExpr::Case` 分支,生成 `CASE WHEN ... THEN ... ELSE ... END`(作为布尔表达式)。

#### 4.3 `QueryState.projected_columns` 扩展

```rust
// Before:
pub projected_columns: Option<Vec<String>>,

// After:
pub projected_columns: Option<Vec<SelectItem>>,

/// SELECT 列表项:列名或标量表达式(带别名)。
#[derive(Debug, Clone)]
pub enum SelectItem {
    /// 简单列引用。
    Column(String),
    /// 标量表达式 + 别名。
    Expr { expr: ScalarExpr, alias: String },
}
```

`to_sql_with` 中 `projected_columns` 处理改为:
```rust
let parts: Vec<String> = cols.iter().map(|item| match item {
    SelectItem::Column(c) => gen.quote_identifier(c),
    SelectItem::Expr { expr, alias } => {
        format!("{} AS {}", expr.to_sql(gen, &mut param_idx), gen.quote_identifier(alias))
    }
}).collect();
```

#### 4.4 宏语法(`crates/macros/src/linq.rs`)

**WHERE 中的 CASE WHEN**(布尔上下文):
```
linq!(ctx.set::<Blog>(), |b: Blog| 
    case when b.rating > 8 then b.featured else b.published end
).to_list().await?
```

**SELECT 中的 CASE WHEN**(投影):
```
linq!(ctx.set::<Blog>();
    select (b.id, case when b.rating > 8 then "high" else "low" end as tier)
).to_list().await?
```

宏解析:`case` 作为新关键字,解析 `case when <expr> then <expr> [when ... then ...]* [else <expr>] end`。

`LinqClause::Select` 扩展:投影项可以是 `expr as alias` 形式。新增 `SelectItemAst`:
```rust
enum SelectItemAst {
    Column(Expr),               // b.id
    Expr { expr: Expr, alias: String },  // case when ... end as tier
}
```

`parse_select_rest` 改为解析 `SelectItemAst` 列表(逗号分隔),识别 `as` 关键字。

#### 4.5 `compile_expr` / `compile_bool_expr` 支持 `case`

新增 `parse_case_expr` 解析 `case when ... then ... else ... end`,生成对应的 `BoolExpr::Case` 或 `ScalarExpr::Case` 代码。

**决策 D7**:本轮 CASE WHEN 仅支持:
- WHERE 中的布尔 CASE(`case when <bool> then <bool> else <bool> end`)
- SELECT 中的标量 CASE(`case when <bool> then <scalar> else <scalar> end as alias`)
- 不支持嵌套标量运算(`a + case when ... end`)— 留作后续增强

### Verification

- 新增 `case_when_tests.rs`:4 个测试
  1. WHERE 中布尔 CASE
  2. SELECT 中标量 CASE + 别名
  3. 多 WHEN 分支
  4. 无 ELSE(CASE 隐式 NULL)
- `cargo test -p rust-ef --test case_when_tests` 全通过

---

## Part 5: 3e UPSERT / MERGE

### What

`DbSet` 新增 `upsert(entity)` API。`EntityState` 新增 `Upsert` 变体。`ChangeExecutor` 新增 `execute_upserts`。`ISqlGenerator` 新增 `upsert` 方法,各方言实现 `ON CONFLICT`(SQLite/PG)/ `ON DUPLICATE KEY UPDATE`(MySQL)。

### Why

原子 upsert 是高并发写入的核心原语(避免"先查后插"竞态)。现有 `add` + `save_changes` 只能 INSERT,遇到唯一约束冲突报错。UPSERT 让框架覆盖 EFCore 的 `AddOrUpdate` 语义,且在 SQL 层原子执行。

### How

#### 5.1 `EntityState` 新增 `Upsert`(`crates/core/src/entity.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityState {
    Detached,
    Added,
    Unchanged,
    Modified,
    Deleted,
    Upsert,  // 新增:INSERT ... ON CONFLICT ... DO UPDATE
}
```

#### 5.2 `DbSet::upsert`(`crates/core/src/db_set.rs`)

```rust
impl<E> DbSet<E> {
    /// 标记实体为 UPSERT:save_changes 时生成 `INSERT ... ON CONFLICT ... DO UPDATE`。
    pub fn upsert(&mut self, entity: E) {
        self.entries.push(EntityEntry::new(entity, EntityState::Upsert));
    }
}
```

#### 5.3 `ISqlGenerator::upsert`(`crates/core/src/provider.rs`)

```rust
pub trait ISqlGenerator: Send + Sync {
    // ... 现有方法 ...
    /// 生成 UPSERT 语句。
    /// - `table`: 表名
    /// - `columns`: 要插入的列
    /// - `conflict_columns`: 冲突检测列(通常为主键/唯一键)
    /// - `update_columns`: 冲突时要更新的列(排除 conflict_columns)
    fn upsert(&self, table: &str, columns: &[&str], conflict_columns: &[&str], update_columns: &[&str]) -> String;
}
```

#### 5.4 方言实现

**SQLite / PostgreSQL**(`ON CONFLICT ... DO UPDATE`):
```rust
fn upsert(&self, table: &str, columns: &[&str], conflict_columns: &[&str], update_columns: &[&str]) -> String {
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| self.parameter_placeholder(i)).collect();
    let col_list = columns.iter().map(|c| self.quote_identifier(c)).collect::<Vec<_>>().join(", ");
    let conflict_cols = conflict_columns.iter().map(|c| self.quote_identifier(c)).collect::<Vec<_>>.join(", ");
    // PostgreSQL 支持 EXCLUDED;SQLite 也支持
    let update_set = update_columns.iter()
        .map(|c| format!("{} = EXCLUDED.{}", self.quote_identifier(c), self.quote_identifier(c)))
        .collect::<Vec<_>>().join(", ");
    format!(
        "INSERT INTO {} ({}) VALUES ({}) ON CONFLICT ({}) DO UPDATE SET {}",
        self.quote_identifier(table), col_list,
        placeholders.join(", "),
        conflict_cols,
        update_set
    )
}
```

**MySQL**(`ON DUPLICATE KEY UPDATE`):
```rust
fn upsert(&self, table: &str, columns: &[&str], conflict_columns: &[&str], update_columns: &[&str]) -> String {
    let _ = conflict_columns;  // MySQL 自动检测唯一键冲突
    let placeholders: Vec<String> = (1..=columns.len()).map(|i| self.parameter_placeholder(i)).collect();
    let col_list = columns.iter().map(|c| self.quote_identifier(c)).collect::<Vec<_>>().join(", ");
    let update_set = update_columns.iter()
        .map(|c| format!("{} = VALUES({})", self.quote_identifier(c), self.quote_identifier(c)))
        .collect::<Vec<_>>().join(", ");
    format!(
        "INSERT INTO {} ({}) VALUES ({}) ON DUPLICATE KEY UPDATE {}",
        self.quote_identifier(table), col_list,
        placeholders.join(", "),
        update_set
    )
}
```

#### 5.5 `ChangeExecutor::execute_upserts`(`crates/core/src/change_executor.rs`)

```rust
pub async fn execute_upserts<E, F>(
    conn: &mut (dyn IAsyncConnection + Send),
    provider: &dyn IDatabaseProvider,
    entries: &mut [EntityEntry<E>],
    meta: &EntityTypeMeta,
) -> EFResult<usize>
where
    E: IEntityType + IEntitySnapshot + IGetKeyValues + Send + Sync,
    F: FnOnce(&mut E),
{
    let gen = provider.sql_generator();
    let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();
    let columns: Vec<&str> = scalar_props.iter().map(|p| p.column_name.as_ref()).collect();
    let pk_columns: Vec<&str> = meta.primary_key_properties().map(|p| p.column_name.as_ref()).collect();
    let update_columns: Vec<&str> = columns.iter()
        .filter(|c| !pk_columns.contains(c))
        .copied()
        .collect();
    
    let sql = gen.upsert(meta.table_name.as_ref(), &columns, &pk_columns, &update_columns);
    let mut count = 0;
    for entry in entries.iter_mut() {
        let params = collect_insert_params(meta, &entry.entity.property_values());
        conn.execute(&sql, &params).await?;
        entry.state = EntityState::Unchanged;
        count += 1;
    }
    Ok(count)
}
```

#### 5.6 `save_changes` 集成

`save_changes` 在遍历 DbSet 时,收集 `EntityState::Upsert` 的条目,调用 `execute_upserts`。`TxnSource` 处理与 INSERT/UPDATE 一致(复用 ambient 或自管事务)。

#### 5.7 宏入口(可选)

`linq!` 宏暂不新增 UPSERT 子句 — UPSERT 是写操作,通过 `DbSet::upsert` + `save_changes` API 使用,不走查询宏。未来可加 `linq!(upsert ctx.set::<Blog>(), blog)` 语法糖。

**决策 D8**:本轮只提供 `DbSet::upsert` API,不扩展 `linq!` 宏。UPSERT 是写操作,与 `linq!` 的查询定位不符。EFCore 的 `AddOrUpdate` 也是 API 而非查询语法。

### Verification

- 新增 `upsert_tests.rs`:4 个测试
  1. 新行 → INSERT 生效
  2. 已有行(主键冲突)→ UPDATE 生效
  3. 批量 upsert(混合新旧行)
  4. 唯一约束(非主键)冲突 → UPDATE
- `cargo test -p rust-ef --test upsert_tests`(SQLite)
- `cargo test -p rust-ef --test upsert_mysql_tests`(MySQL,若环境可用)

---

## Assumptions & Decisions

| ID | 决策 | 理由 |
|----|------|------|
| D1 | `ITransaction::commit`/`rollback` 消费 `self: Box<Self>` | 保证用后即弃,避免双重提交 |
| D2 | 不在 `Drop` 中实际回滚(async Drop 不可行) | 文档要求显式 commit/rollback 或用 `use_transaction` |
| D3 | `begin_transaction` 返回句柄不注册 ambient;`use_transaction` 注册 ambient | 分离手动控制与作用域控制,职责清晰 |
| D4 | 接受 `Box::pin(async move)` 调用方样板 | Rust async + 借用检查器的必要代价 |
| D5 | 集合运算 operand 不含 ORDER BY/LIMIT(文档说明) | 简化首轮实现;operand ORDER BY 需括号包裹 |
| D6 | MySQL FULL OUTER JOIN 不模拟(报 SQL 错误) | 模拟需 UNION 两边 LEFT JOIN,复杂度高;留作后续 |
| D7 | CASE WHEN 仅支持 WHERE 布尔 + SELECT 标量 | 嵌套标量运算留作后续 |
| D8 | UPSERT 仅 API,不扩展 `linq!` 宏 | UPSERT 是写操作,与查询宏定位不符 |

### 假设

- `QueryBuilder` 有 `provider` 字段(`Arc<dyn IDatabaseProvider>`),可 clone — **需验证**(若无则新增 `new_with_provider`)
- `EntityEntry` 有 `entity` 字段且可 `property_values()` — **需验证**
- `EntityTypeMeta` 有 `primary_key_properties()` 方法 — **需验证**
- 各方言 `ISqlGenerator` 实现可独立扩展 — 已确认(sql_generator.rs 各自独立)

---

## Verification Steps(全局)

1. `cargo build -p rust-ef -p rust-ef-sqlite -p rust-ef-postgres -p rust-ef-mysql` — 全编译通过
2. `cargo test -p rust-ef` — 全测试通过(postgres_crud_tests 环境问题除外)
3. `cargo test -p rust-ef-sqlite` — 全通过
4. 新增测试文件:
   - `transaction_scope_tests.rs`(Part 0)
   - `set_op_tests.rs`(Part 1)
   - `recursive_cte_tests.rs`(Part 2)
   - `linq_dsl_tests.rs` 扩展(Part 3)
   - `case_when_tests.rs`(Part 4)
   - `upsert_tests.rs`(Part 5)
5. `CHANGELOG.md` 更新:`### Added — linq coverage extension (priority 3)` + `### Changed — ITransaction abstraction`
6. `crates/core/src/lib.rs` prelude 导出 `ITransaction`、`SetOperator`、`ScalarExpr`、`SelectItem`

## 实施顺序

1. **Part 0**(ITransaction)— 重构 Priority 2,为后续提供稳定基础
2. **Part 3**(JOIN 类型)— 最简单,快速验证 `JoinSpec` 扩展模式
3. **Part 1**(集合运算)— 中等复杂度,验证 `QueryState` 扩展模式
4. **Part 2**(递归 CTE)— 基于 Part 1 的 CTE 理解
5. **Part 4**(CASE WHEN)— AST 扩展,影响面大
6. **Part 5**(UPSERT)— 独立的写路径,不影响查询

每个 Part 完成后立即运行对应测试,确保增量可验证。
