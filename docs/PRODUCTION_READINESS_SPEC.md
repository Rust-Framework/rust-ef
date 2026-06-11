# lref 生产就绪技术规格说明书

> 版本: v0.2 - 基于 2026-06-11 审计结果  
> 目标: 逐步推进至 v1.0 生产就绪状态  
> 当前阶段: Alpha 2 (35-40% 就绪度)

---

## 里程碑总览

```
Alpha 2 ──► Beta 1 ──► RC 1 ──► 1.0
 (当前)     (6周)     (4周)    (2周)
```

---

# 里程碑一：Beta 1 — CRUD 完整链路 + 查询完备性

**目标**: 用户可以用 lref 完成任意实体的增删改查，无需手写 SQL

## 1.1 通用 SaveChanges 实现

### 现状
- `DbContext::save_changes()` 有 trait 定义，但 **无默认实现**
- Blog 示例中手动为 Blog/Post 写了 INSERT，完全不通用
- 每新增一个实体类型，用户必须手动实现 `save_changes_impl()`

### 需求规格

**1.1.1 为 `DbContext` 提供默认 `save_changes()` 实现**

文件: `lref/src/db_context.rs`

```rust
#[async_trait::async_trait]
pub trait DbContext: Send + Sync + Sized {
    // ... 现有方法 ...

    /// 默认实现：自动为所有 DbSet 中的变更生成并执行 INSERT/UPDATE/DELETE
    async fn save_changes(&mut self) -> LrefResult<SaveChangesResult> {
        self.save_changes_with_strategy(SaveStrategy::Transactional).await
    }
}
```

**1.1.2 引入 `DbSetCollection` — 注册所有 DbSet 的容器**

文件: `lref/src/db_context.rs` (新增模块)

```rust
/// 用户通过宏或 builder 将所有 DbSet 注册到此容器
pub struct DbSetCollection {
    sets: Vec<Box<dyn AnyDbSet>>,
}

impl DbSetCollection {
    pub fn register<T: EntityType + FromRow + GetKeyValues>(
        &mut self,
        db_set: &DbSet<T>,
    ) { /* ... */ }
}
```

**1.1.3 引入 `GetKeyValues` trait（derive 宏自动生成）**

文件: `lref/src/entity.rs`

```rust
/// 提取实体的主键值，供 SaveChanges 生成 WHERE 条件
pub trait GetKeyValues: EntityType {
    /// 返回 HashMap<column_name, DbValue>
    fn key_values(&self) -> HashMap<String, DbValue>;
}
```

**1.1.4 引入 `EntitySnapshot` trait — 提取所有标量属性值**

```rust
pub trait EntitySnapshot: EntityType {
    /// 返回所有标量属性的当前值
    fn snapshot(&self) -> HashMap<String, DbValue>;
}
```

**1.1.5 实现 `ChangeExecutor` — 自动生成并执行 DML**

文件: `lref/src/change_executor.rs` (重写)

功能:
- 遍历 `DbSetCollection` 中的所有 DbSet
- 对 `EntityState::Added` 的实体 → 生成参数化 INSERT，执行后通过 RETURNING 获取生成的主键
- 对 `EntityState::Modified` 的实体 → 生成 UPDATE SET ... WHERE pk=?
- 对 `EntityState::Deleted` 的实体 → 生成 DELETE WHERE pk=?
- 全部在 Provider 事务中完成

### 验收标准
- [ ] 用户定义新实体类型后，`ctx.save_changes()` 能自动持久化所有变更
- [ ] INSERT 后实体主键（如 `blog_id`）被正确回填
- [ ] 事务回滚场景正确（任一步骤失败，全部回滚）
- [ ] 集成测试：SQLite 内存数据库中 CRUD 整个实体生命周期

---

## 1.2 条件表达式完善

### 现状
- 仅支持单一 `AND` 连接的条件
- 无 `OR`、`IN`、`NOT`、`BETWEEN`、`IS NULL`（独立语法）

### 需求规格

**1.2.1 表达式树代替线性过滤器列表**

文件: `lref/src/query.rs`

```rust
/// 布尔表达式 AST
#[derive(Debug, Clone)]
pub enum BoolExpr {
    /// 单个条件
    Filter(FilterCondition),
    /// AND 组合
    And(Box<BoolExpr>, Box<BoolExpr>),
    /// OR 组合
    Or(Box<BoolExpr>, Box<BoolExpr>),
    /// NOT 取反
    Not(Box<BoolExpr>),
}
```

**1.2.2 新增 QueryBuilder 方法**

```rust
// OR 条件
pub fn or(mut self, f: impl FnOnce(QueryBuilder<T>) -> QueryBuilder<T>) -> Self;

// IN 查询
pub fn filter_in(mut self, column: &str, values: Vec<impl Into<DbValue>>) -> Self;

// NOT 条件
pub fn filter_not(mut self, column: &str, operator: &str, value: impl Into<DbValue>) -> Self;

// IS NULL / IS NOT NULL
pub fn filter_is_null(mut self, column: &str) -> Self;
pub fn filter_is_not_null(mut self, column: &str) -> Self;

// BETWEEN
pub fn filter_between(mut self, column: &str, low: impl Into<DbValue>, high: impl Into<DbValue>) -> Self;
```

### 验收标准
- [ ] `WHERE (a = 1 OR a = 2) AND b > 3` 正确生成 SQL
- [ ] `WHERE id IN (1, 2, 3)` 使用正确的参数化占位符
- [ ] 15+ 个表达式组合的单元测试全部通过

---

## 1.3 缺失的 DDL / 辅助操作

### 需求规格

**1.3.1 DbSet 级方法**

```rust
// 批量删除
pub async fn remove_range(&mut self, entities: &[T]) -> LrefResult<()>;

// 从数据库加载（替换 DbSet 内容）
pub async fn load_all(&mut self) -> LrefResult<()>;

// 检查是否存在（不返回实体）
pub async fn exists_by_id(&self, id_values: HashMap<String, DbValue>) -> LrefResult<bool>;
```

**1.3.2 DbContext 级 DDL 方法**

```rust
// 创建表
async fn ensure_created(&self) -> LrefResult<()>;
// 删除表
async fn ensure_deleted(&self) -> LrefResult<()>;
```

### 验收标准
- [ ] 集成测试验证 `ensure_created` → CRUD → `ensure_deleted` 完整流程

---

## 1.4 测试：SQLite 集成测试套件

### 需求规格

文件: `lref/tests/sqlite_crud_tests.rs`

测试场景:
1. **CRUD 生命周期**: define entity → create table → insert → query → update → query → delete → query → drop table
2. **事务回滚**: insert → rollback → query 验证不存在
3. **并发**: 多个实体同时变更的 save_changes
4. **复合主键**: UserRole 实体的完整 CRUD
5. **空表查询**: to_list 返回空 Vec
6. **类型映射**: i32/i64/String/Option/f64/bool 读写一致

### 验收标准
- [ ] 6 个集成测试全部通过
- [ ] 测试不依赖外部数据库（使用 SQLite `:memory:`）

---

# 里程碑二：RC 1 — 导航/高级特性全功能就绪

## 2.1 Eager Loading 导航物化

### 现状
- `include_with_join()` 只生成 JOIN SQL
- 查询结果中的导航属性（`blog.posts`, `post.blog`）永远是默认值

### 需求规格

**2.1.1 双查询策略（避免 Cartesian Product）**

```rust
/// 执行查询并自动填充 Include 指定的导航属性
pub async fn to_list_with_includes(self) -> LrefResult<Vec<T>>
```

实现步骤:
1. 执行主查询，获取主实体集合
2. 收集所有主实体的主键值
3. 对每个 `IncludePath`，执行 `SELECT * FROM related_table WHERE fk IN (...)` 查询
4. 将子实体按外键分组，填充到主实体的导航属性中

**2.1.2 导航属性写入 trait**

```rust
/// Trait for setting navigation property values after lazy/eager loading.
pub trait Navigable<T: EntityType>: EntityType {
    fn set_navigation(&mut self, field_name: &str, value: Box<dyn Any + Send>);
}
```

由 derive 宏自动生成，按字段名匹配到正确的导航属性（`HasMany<T>`/`BelongsTo<T>`/`HasOne<T>`）。

### 验收标准
- [ ] `ctx.blogs.query().include_named("posts").to_list()` 返回的 Blog 中 `posts` 非空
- [ ] `ctx.posts.query().include_named("blog").to_list()` 返回的 Post 中 `blog` 非空
- [ ] 嵌套 Include 支持：`include("posts").then_include("comments")`

---

## 2.2 全局查询过滤器自动注入

### 现状
- `ModelBuilder::has_query_filter()` 注册成功
- `QueryBuilder` 完全不知道过滤器的存在

### 需求规格

**2.2.1 查询时自动注入**

```rust
// QueryBuilder 构造时（via DbSet::query()）自动检查 ModelBuilder
pub fn query(&self) -> QueryBuilder<T> {
    let mut qb = match &self.provider {
        Some(p) => QueryBuilder::with_provider(&self.table_name, p.clone()),
        None => QueryBuilder::new(&self.table_name),
    };
    // 自动注入全局过滤器
    if let Some(filter) = self.model_builder.get_query_filter(&TypeId::of::<T>()) {
        qb.state.filters.push(FilterCondition::raw(filter));
    }
    qb
}
```

**2.2.2 FilterCondition 支持原始 SQL**

```rust
impl FilterCondition {
    /// 创建原始 SQL 条件（无参数占位符）
    pub fn raw(expression: impl Into<String>) -> Self { /* ... */ }
}
```

### 验收标准
- [ ] 注册 `has_query_filter::<Blog>("is_deleted = false")` 后
- [ ] 所有 `ctx.blogs.query().to_list()` 自动包含 `WHERE is_deleted = false`

---

## 2.3 乐观并发控制生效

### 现状
- `#[concurrency_check]` 属性收集到元数据
- UPDATE 语句未做原始值检查

### 需求规格

**2.3.1 SaveChanges 时检查并发令牌**

```rust
// 对于 Modified 状态的实体：
// 1. 读取当前数据库中的行版本/时间戳
// 2. 与 ChangeTracker 中的原始快照比较
// 3. 如果不一致 → 返回 LrefError::ConcurrencyConflict
// 4. 一致 → 执行 UPDATE + 更新令牌值
```

**2.3.2 ChangeTracker 快照扩展**

```rust
struct TrackerEntry {
    // ... 现有字段 ...
    /// 用于并发检查的令牌快照
    concurrency_tokens: HashMap<String, String>,
}
```

### 验收标准
- [ ] 两个并发连接修改同一实体，第二个收到 `ConcurrencyConflict` 错误
- [ ] 乐观并发检查在 Update 时使用 `WHERE token_column = @original_value`

---

## 2.4 CLI Migration 连接数据库

### 现状
- `lref migration apply` 只维护 `.history` 文件
- 不能连接数据库执行 SQL

### 需求规格

```rust
// CLI apply 接受 --connection 参数
// 连接数据库 → 读取 __ef_migrations_history 表 → 执行未应用的迁移
lref migration apply --connection "postgres://localhost/blogging"
```

实现:
1. 使用 Provider 连接数据库
2. 创建 `__ef_migrations_history` 表（如果不存在）
3. 读取已应用的迁移列表
4. 按顺序执行未应用迁移的 `up.sql`
5. 记录到 `__ef_migrations_history`

### 验收标准
- [ ] `lref migration init` 在数据库中创建历史表
- [ ] `lref migration apply` 在 PostgreSQL 中成功执行迁移 SQL

---

# 里程碑三：1.0 — 文档 / CI / 性能 / 安全

## 3.1 完整用户文档

### 需求
- **快速入门**: 5 分钟从零到第一个 CRUD 应用
- **实体建模指南**: 所有属性的详细说明 + 复合主键 + 并发令牌
- **查询参考**: 每个 QueryBuilder 方法的独立文档 + 示例
- **Provider 配置**: PostgreSQL/MySQL/SQLite 连接字符串格式
- **迁移指南**: CLI 命令完整参考
- **最佳实践**: 性能优化 / 事务使用 / 连接池配置

产出: `docs/book/` 目录下的 mdBook 项目

---

## 3.2 CI/CD Pipeline

文件: `.github/workflows/ci.yml`

```yaml
jobs:
  test:
    strategy:
      matrix:
        db: [sqlite, postgres, mysql]
    services:
      postgres:
        image: postgres:16
        env:
          POSTGRES_PASSWORD: test
    steps:
      - cargo test --all-features
      - cargo clippy -- -D warnings
      - cargo fmt --check
```

---

## 3.3 查询结果缓存层

### 需求

```rust
// 可选的二级缓存
impl<T: EntityType> DbSet<T> {
    /// 尝试从 Identity Map 获取实体，未命中时查询数据库
    pub async fn find_cached(
        &self,
        key: &HashMap<String, DbValue>,
    ) -> LrefResult<Option<T>> { /* ... */ }
}
```

---

## 3.4 性能基准

### 需求

文件: `benches/crud_benchmark.rs`

- 批量插入 1000 条实体的吞吐量
- QueryBuilder `to_list()` 延迟
- Eager Loading 的 N+1 避免验证
- 与 `diesel` / `sqlx` 的对比数据

---

# 验收矩阵

| 能力 | Alpha 2 状态 | Beta 1 目标 | RC 1 目标 | 1.0 目标 |
|------|:-----------:|:----------:|:--------:|:-------:|
| 通用 SaveChanges | 手写 | 自动 | 自动+主键回填 | 自动+并发检查 |
| WHERE 表达式 | AND only | AND/OR/IN/NOT/IS NULL/BETWEEN | 子查询 | — |
| 导航 Eager Loading | SQL only | SQL only | 物化+嵌套 | 缓存 |
| 全局过滤器 | 注册 | 注册 | 自动注入 | — |
| 乐观并发 | 元数据 | 元数据 | 生效 | 测试覆盖 |
| CLI Migration | 本地文件 | 本地文件 | 连接数据库 | SQLite/PG/MySQL |
| 测试覆盖 | 19 tests | 25+ tests | 30+ tests | 50+ tests |
| 文档 | 计划文档 | API 文档 | 用户指南 | 完整 mdBook |

---

# 实现优先级（按顺序）

```
Beta 1:
  1.1 通用 SaveChanges          ← 最高优先级，解锁所有后续工作
  1.2 表达式完善 (OR/IN/NOT)     ← 查询可用性的基本要求
  1.3 DDL 辅助方法               ← 配合集成测试
  1.4 SQLite 集成测试            ← 验证 1.1-1.3 的正确性

RC 1:
  2.1 Eager Loading 物化         ← 关系 ORM 的核心价值
  2.2 全局过滤器自动注入         ← 多租户/软删除的前提
  2.3 乐观并发生效               ← 并发场景安全
  2.4 CLI Migration 连接DB       ← 生产部署的前提

1.0:
  3.1 文档                       ← 用户采纳的关键
  3.2 CI/CD                      ← 持续质量保障
  3.3 缓存层                     ← 性能优化
  3.4 性能基准                   ← 竞争分析
```
