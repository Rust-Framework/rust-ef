# 级联删除实现计划

## Context

级联保存（INSERT/UPDATE/DELETE 拓扑排序）已实现，但当前 DELETE 阶段只删除显式标记为 Deleted 的实体。当删除主实体（如 Blog）时，其依赖实体（如 Posts）不会自动删除——用户必须手动标记每个 Post 为 Deleted。

EFCore 的级联删除行为：标记主实体 Deleted 时，自动标记已加载的依赖实体为 Deleted，并对未加载的依赖实体发出直接 DELETE SQL。`DeleteBehavior` 枚举已定义（`crates/core/src/relations.rs:415`）但从未使用。

## 当前状态

| 组件 | 状态 |
|------|------|
| `DeleteBehavior` 枚举 | ✅ 已定义（Cascade/Restrict/SetNull/NoAction），❌ 从未使用 |
| `NavigationMeta.delete_behavior` | ❌ 不存在 |
| 级联删除 drain 循环 | ❌ 不存在（当前 drain 循环只处理 Added） |
| 直接 DELETE SQL | ❌ 不存在 |
| FK DDL `ON DELETE` 子句 | ❌ 不存在 |
| `#[on_delete(...)]` 属性 | ❌ 不存在 |
| `fk_reference_for_property` | ⚠️ 按 `foreign_key_field` 匹配，可能返回 None |

## Proposed Changes

### Step 1: NavigationMeta 添加 delete_behavior 字段
**文件**: [metadata.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/metadata.rs)

在 `NavigationMeta`（line 164-201）添加：
```rust
pub delete_behavior: Option<DeleteBehavior>,
```
添加 `use crate::relations::DeleteBehavior;` 导入。

### Step 2: 宏解析 #[on_delete] 属性
**文件**: [entity.rs](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs)

1. 添加 `extract_on_delete` 辅助函数（返回 `Option<DeleteBehavior>` token stream）
2. 在 3 处 `NavigationMeta { ... }` 构造（line 132/165/187）添加 `delete_behavior: #delete_behavior_tokens`
3. 在 [lib.rs](file:///d:/GitCode/RF/rust-ef/crates/macros/src/lib.rs) 注册 `on_delete` 为 derive 属性

### Step 3: 新增 CascadeDeleteDirective 类型
**文件**: [cascade.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/cascade.rs)

```rust
pub enum CascadeDeleteAction { Delete, SetNull }

pub struct CascadeDeleteDirective {
    pub table: String,
    pub fk_column: String,
    pub principal_pk: i64,
    pub action: CascadeDeleteAction,
}
```

### Step 4: ErasedSetOps 新增级联删除方法
**文件**: [db_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs)

新增 2 个 trait 方法 + 实现：

**`drain_cascade_deleted_children`** — 遍历 Deleted 实体，对每个 HasMany/M2M 导航：
- M2M：收集 `CascadeDeleteDirective { action: Delete, table: through_table, fk_column: through_parent_fk }`（删除连接表行，不删除关联实体）
- HasMany + Cascade：drain 已加载的子实体（`drain_has_many`），同时收集 DELETE directive（处理未加载的子实体）
- HasMany + SetNull：收集 SetNull directive，不 drain
- HasMany + Restrict/NoAction：跳过
- 用 `processed: HashSet<(TypeId, usize)>` 避免重复处理

**`add_cascade_deleted_child`** — 将 drained 子实体以 `EntityState::Deleted` 添加到子 DbSet

**DeleteBehavior 默认解析**：`None` 时，M2M→Cascade，required FK→Cascade，optional FK→Restrict

### Step 5: save_changes 集成级联删除
**文件**: [db_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs)

在级联 INSERT drain 循环之后、拓扑排序之前，添加级联 DELETE drain 循环（与 INSERT drain 结构相同，但处理 Deleted 实体）。

在 DELETE 阶段开始前（PK-based delete 之前），执行直接 DELETE/SET NULL SQL：
```
DELETE FROM child_table WHERE fk_column = ?    -- Cascade
UPDATE child_table SET fk_column = NULL WHERE fk_column = ?  -- SetNull
```

### Step 6: FK DDL 添加 ON DELETE 子句
**文件**: [migration.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration.rs)

1. `fk_reference_for_property` 改为按 `fk_column` 匹配（当前按 `foreign_key_field` 匹配，可能不匹配），返回 3-tuple 含 ON DELETE 子句
2. `SnapshotColumn` 添加 `fk_on_delete: Option<String>`
3. `SchemaChange::AddForeignKey` 添加 `on_delete: Option<String>` 字段
4. `generate_up_sql_inner` 的 `AddForeignKey` 分支追加 `ON DELETE {clause}`
5. `columns_structurally_equal` 添加 `fk_on_delete` 比较
6. 更新 `snapshot_to_json` / `snapshot_from_json` 序列化新字段

⚠️ **风险**：修复 `fk_reference_for_property` 可能让之前没有 FK 约束的 DDL 突然生成 FK 约束，破坏现有测试。需验证。

### Step 7: 测试用例
**文件**: [cascade_save_tests.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/cascade_save_tests.rs)

新增 6 个测试：
1. `cascade_delete_loaded_children` — 加载 Blog+Posts(Include)，标记 Blog Deleted，验证 Posts 自动删除
2. `cascade_delete_untracked_children` — 不 Include，标记 Blog Deleted，验证直接 DELETE SQL 删除 Posts
3. `cascade_delete_m2m_join_rows` — 标记 Student Deleted，验证 Enrollment 连接行删除，Course 保留
4. `cascade_delete_nested` — Blog→Post→Comment 三级 Include，标记 Blog Deleted，验证全部删除
5. `cascade_delete_self_referential` — Category 树，标记 Root Deleted，验证整树删除
6. `cascade_delete_set_null` — OptionalBlog + OptionalPost（nullable FK, `#[on_delete(SetNull)]`），标记 Blog Deleted，验证 Post 保留且 FK=NULL

现有 `cascade_delete_reverse_order` 测试应继续通过（手动标记 Blog+Post 为 Deleted，cascade drain 无额外操作，直接 DELETE 处理未加载部分）。

## ⚠️ 需要提醒的问题

1. **直接 DELETE 不应用查询过滤器**：`DELETE WHERE fk = ?` 不含租户隔离等过滤器。已加载的子实体（PK-based delete）会应用过滤器。多租户场景应先 Include 再删除。
2. **未加载的深层级联**：直接 DELETE 只处理一层。Blog→Post→Comment 全未加载时，只删除 Posts，Comments 需要 DB 级 `ON DELETE CASCADE`（Step 6 的 DDL 增强）或用户手动加载。
3. **SQLite FK 执行**：SQLite 默认 `PRAGMA foreign_keys = OFF`，DB 级 ON DELETE CASCADE 不生效。应用级级联（Steps 1-5）不受影响。
4. **重复删除**：已加载子实体被 PK-based delete 删除后，直接 DELETE 找不到行（0 rows affected），无害。
5. **`fk_reference_for_property` 修复风险**：可能改变现有 DDL 输出，需验证现有迁移测试不破坏。

## Verification

1. `cargo check --workspace` — 全 crate 编译通过
2. `cargo test -p rust-ef --test cascade_save_tests` — 6 个新测试 + 6 个现有测试全部通过
3. `cargo test -p rust-ef` — 全量回归（除 PostgreSQL 连接测试）
4. 重点验证：现有 `cascade_delete_reverse_order` 仍通过（手动标记两个实体 Deleted 的场景）

## 实施顺序

1. Steps 1-2（元数据 + 宏）→ `cargo check -p rust-ef-macros`
2. Step 3（类型定义）→ `cargo check -p rust-ef`
3. Step 4（ErasedSetOps 方法）→ `cargo check -p rust-ef`
4. Step 5（save_changes 集成）→ `cargo check -p rust-ef`
5. Step 7（测试）→ `cargo test --test cascade_save_tests`
6. Step 6（FK DDL，最后做以降低风险）→ `cargo test -p rust-ef`
