# 级联删除后续任务实施计划

## Context

级联删除 Step 1-4 已基本完成：`NavigationMeta.delete_behavior` 字段已添加、`#[on_delete]` 宏属性已解析、`CascadeDeleteDirective` 类型已定义、`ErasedSetOps` 的两个新方法（`drain_cascade_deleted_children` / `add_cascade_deleted_child`）已实现。

**当前阻塞点**：`drain_cascade_deleted_children` 实现在 [db_context.rs:708](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L708) 调用了 `resolve_delete_behavior(nav)`，但该函数尚未定义，导致编译失败。

本计划完成剩余的 Step 4（补全缺失函数）、Step 5（save_changes 集成）、Step 7（测试用例）、Step 6（FK DDL ON DELETE 子句，最后做以降低风险）。

## 当前状态

| 组件 | 状态 |
|------|------|
| Step 1-3（元数据/宏/类型） | ✅ 已完成 |
| Step 4 `drain_cascade_deleted_children` / `add_cascade_deleted_child` | ✅ 已实现（[db_context.rs:658-794](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L658)） |
| Step 4 `resolve_delete_behavior` 函数 | ❌ 缺失，编译失败 |
| Step 5 save_changes 级联 DELETE drain 循环 | ❌ 未实现 |
| Step 5 直接 DELETE/SET NULL SQL 执行 | ❌ 未实现 |
| Step 6 FK DDL ON DELETE 子句 | ❌ 未实现 |
| Step 7 测试用例（6 个） | ❌ 未实现 |

## Proposed Changes

### Step 4 补全：添加 `resolve_delete_behavior` 自由函数

**文件**: [db_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs)

在 `impl<E> ErasedSetOps for SetOps<E>` 块结束后（约 [line 795](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L795)）、`DbContext` 结构体定义之前，添加自由函数：

```rust
fn resolve_delete_behavior(nav: &crate::metadata::NavigationMeta) -> crate::relations::DeleteBehavior {
    use crate::relations::DeleteBehavior;
    if let Some(b) = nav.delete_behavior {
        return b;
    }
    if nav.kind == crate::metadata::NavigationKind::ManyToMany {
        return DeleteBehavior::Cascade;
    }
    if let Some(meta_fn) = nav.related_entity_meta {
        let child_meta = meta_fn();
        if let Some(fk_prop) = child_meta.properties.iter().find(|p| p.is_foreign_key) {
            return if fk_prop.is_required {
                DeleteBehavior::Cascade
            } else {
                DeleteBehavior::Restrict
            };
        }
    }
    DeleteBehavior::Cascade
}
```

**验证**: `cargo check -p rust-ef` 编译通过。

---

### Step 5：save_changes 集成级联删除

**文件**: [db_context.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs)

#### 5a. 级联 DELETE drain 循环

在现有级联 INSERT drain 循环之后（[line 1175](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L1175) `}` 之后）、拓扑排序（[line 1177](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L1177)）之前，插入级联 DELETE drain 循环。

结构与 INSERT drain 循环相同，但：
- 调用 `saver.drain_cascade_deleted_children(set.as_mut(), meta, &mut processed)` 替代 `drain_cascade_children`
- 用 `processed: HashSet<(TypeId, usize)>` 避免重复处理（跨循环迭代）
- 返回 `(Vec<DrainedChild>, Vec<CascadeDeleteDirective>)`
- 对每个 drained child，调用 `child_saver.add_cascade_deleted_child(child_set.as_mut(), child.child)` 将其以 `EntityState::Deleted` 添加到子 DbSet
- 收集所有 `CascadeDeleteDirective` 到 `delete_directives: Vec<CascadeDeleteDirective>`

```rust
// --- Cascade DELETE drain loop ---
// Iteratively drain HasMany children from Deleted principals. Drained
// children are added to their target DbSet as Deleted. Also collects
// direct DELETE/SET NULL directives for untracked dependents.
let mut delete_directives: Vec<CascadeDeleteDirective> = Vec::new();
let mut processed: std::collections::HashSet<(TypeId, usize)> = std::collections::HashSet::new();
loop {
    let mut all_drained_deleted: Vec<DrainedChild> = Vec::new();
    for type_id in &type_ids {
        let saver = self.savers.get(type_id).expect("saver not registered");
        let set = self.sets.get_mut(type_id).unwrap();
        let meta = configured_metas.get(type_id).or_else(|| self.entity_metas.get(type_id)).expect("meta not found");
        let (drained, directives) = saver.drain_cascade_deleted_children(set.as_mut(), meta, &mut processed);
        all_drained_deleted.extend(drained);
        delete_directives.extend(directives);
    }
    if all_drained_deleted.is_empty() {
        break;
    }
    for child in all_drained_deleted {
        let child_saver = self.savers.get(&child.child_type_id).ok_or_else(|| {
            EFError::configuration(format!(
                "Cannot cascade-delete child type {:?}: no DbSet registered.",
                child.child_type_id
            ))
        })?;
        let child_set = self.sets.get_mut(&child.child_type_id).expect("set not found");
        child_saver.add_cascade_deleted_child(child_set.as_mut(), child.child);
    }
}
```

#### 5b. 直接 DELETE/SET NULL SQL 执行

在 DELETE 阶段循环（[line 1446](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L1446) `// --- DELETE phase`）之前，执行收集到的 `delete_directives`。

每个 directive 生成一条 SQL：
- `CascadeDeleteAction::Delete` → `DELETE FROM {table} WHERE {fk_column} = ?`
- `CascadeDeleteAction::SetNull` → `UPDATE {table} SET {fk_column} = NULL WHERE {fk_column} = ?`

参数为 `[DbValue::from(principal_pk)]`。使用 `conn_ref.execute(&sql, &params).await`（参考 [line 1347-1367](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L1347) M2M insert 的错误处理模式）。

```rust
// --- Direct cascade DELETE/SET NULL SQL (before PK-based deletes) ---
for directive in &delete_directives {
    let sql = match directive.action {
        CascadeDeleteAction::Delete => format!(
            "DELETE FROM {} WHERE {} = ?",
            directive.table, directive.fk_column
        ),
        CascadeDeleteAction::SetNull => format!(
            "UPDATE {} SET {} = NULL WHERE {} = ?",
            directive.table, directive.fk_column, directive.fk_column
        ),
    };
    let params = vec![DbValue::from(directive.principal_pk)];
    let conn_ref: &mut dyn IAsyncConnection = match &mut txn {
        TxnSource::Ambient(t) => t.connection(),
        TxnSource::Managed(c) => c.as_mut(),
    };
    if let Err(e) = conn_ref.execute(&sql, &params).await {
        // 错误处理：回滚/恢复 ambient + interceptor + return Err
        // （复用 M2M insert 的错误处理模式）
    }
}
```

**需导入**: 在 [line 60](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L60) 的 `use crate::cascade::{...}` 中，`CascadeDeleteAction` 和 `CascadeDeleteDirective` 已导入（已确认）。

**验证**: `cargo check -p rust-ef` 编译通过。

---

### Step 7：测试用例（6 个）

**文件**: [cascade_save_tests.rs](file:///d:/GitCode/RF/rust-ef/crates/core/tests/cascade_save_tests.rs)

在现有 `cascade_save_tests` mod 末尾（[line 362](file:///d:/GitCode/RF/rust-ef/crates/core/tests/cascade_save_tests.rs#L362) `}` 之前）追加 6 个测试。

#### 新增实体定义

为避免修改现有实体（破坏现有测试），新增专用实体：

```rust
// ── Nested cascade entities (Blog → Post → Comment) ──
#[derive(Debug, Clone, EntityType)]
#[table("cascade_nest_blogs")]
struct CascadeNestBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    url: String,
    #[navigation]
    posts: HasMany<CascadeNestPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_nest_posts")]
struct CascadeNestPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(CascadeNestBlog)]
    blog_id: i32,
    #[navigation]
    blog: BelongsTo<CascadeNestBlog>,
    #[navigation]
    comments: HasMany<CascadeNestComment>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_nest_comments")]
struct CascadeNestComment {
    #[primary_key]
    #[auto_increment]
    comment_id: i32,
    #[required]
    text: String,
    #[foreign_key(CascadeNestPost)]
    post_id: i32,
}

// ── SetNull cascade entities (nullable FK + #[on_delete(SetNull)]) ──
#[derive(Debug, Clone, EntityType)]
#[table("cascade_optional_blogs")]
struct CascadeOptionalBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    url: String,
    #[navigation]
    #[on_delete(SetNull)]
    posts: HasMany<CascadeOptionalPost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_optional_posts")]
struct CascadeOptionalPost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(CascadeOptionalBlog)]
    blog_id: Option<i32>,  // nullable FK
    #[navigation]
    blog: BelongsTo<CascadeOptionalBlog>,
}
```

#### 测试模式（关键设计决策）

为避免双重追踪问题（insert 后实体已 tracked，再 attach 同 PK 实体导致重复），采用**双 Context 模式**：
1. ctx1：插入数据，save_changes
2. ctx2（全新）：查询带 include 的实体，attach 为 Deleted，save_changes 验证级联删除

#### 6 个测试

1. **`cascade_delete_loaded_children`** — 复用 CascadeBlog/CascadePost
   - ctx1 插入 Blog+2 Posts，save
   - ctx2: `query().include_internal("posts").to_list()` → 加载 blog 带 posts
   - `ctx2.set::<CascadeBlog>().attach(loaded_blog)` → Unchanged
   - `ctx2.set::<CascadeBlog>().remove_at(0)` → Deleted（HasMany 保留）
   - save_changes → cascade drain 提取 posts，标记 Deleted
   - 验证 blogs 表空、posts 表空

2. **`cascade_delete_untracked_children`** — 复用 CascadeBlog/CascadePost
   - ctx1 插入 Blog+2 Posts，save
   - ctx2: `query().to_list()`（不 include）→ blog 的 HasMany 为空
   - attach blog + remove_at(0) → Deleted
   - save_changes → drain 无 children，但直接 DELETE SQL `DELETE FROM cascade_posts WHERE blog_id = ?` 删除未加载 posts
   - 验证 blogs 表空、posts 表空

3. **`cascade_delete_m2m_join_rows`** — 复用 CascadeStudent/CascadeCourse/CascadeEnrollment
   - ctx1 插入 Student+Course+Enrollment，save
   - ctx2: 查询 student（不 include courses），attach + remove_at → Deleted
   - save_changes → M2M 直接 DELETE `DELETE FROM cascade_enrollments WHERE student_id = ?`
   - 验证 enrollments 表空、courses 表保留、students 表空

4. **`cascade_delete_nested`** — 使用 CascadeNestBlog/CascadeNestPost/CascadeNestComment
   - ctx1 插入 Blog→Post→Comment（三级），save
   - ctx2: `query().include_internal("posts").then_include("comments")` 或等效链式 include
   - attach blog + remove_at → Deleted
   - save_changes → cascade drain 提取 posts（带 comments），递归 drain 提取 comments
   - 验证三表全空

5. **`cascade_delete_self_referential`** — 复用 CascadeCategory
   - ctx1 插入 Root→Child A/B（二级），save
   - ctx2: `query().include_internal("children")` → 加载 Root 带 children
   - attach root + remove_at → Deleted
   - save_changes → cascade drain 提取 children
   - 验证 categories 表空

6. **`cascade_delete_set_null`** — 使用 CascadeOptionalBlog/CascadeOptionalPost
   - ctx1 插入 Blog+Post，save
   - ctx2: 查询 blog（不 include），attach + remove_at → Deleted
   - save_changes → 直接 SET NULL SQL `UPDATE cascade_optional_posts SET blog_id = NULL WHERE blog_id = ?`
   - 验证 blogs 表空、posts 表保留 1 行且 blog_id 为 NULL

#### 注意事项

- **`include_internal` 链式 include**: 需确认 `then_include` 或嵌套 include API 是否存在。若不存在，测试 4 可改为分别查询并手动组装，或使用单层 include 验证二级删除（Blog→Post），Comment 通过直接 DELETE SQL 删除。实施时先验证 API。
- **`#[on_delete(SetNull)]`**: 需确认宏已正确解析（Step 2 已完成 `extract_on_delete`）。SetNull 要求 FK 列可为 NULL（`Option<i32>`）。
- **现有 `cascade_delete_reverse_order` 测试**: 手动标记 Blog+Post 为 Deleted。cascade DELETE drain 会处理 Blog 的 HasMany（已被 INSERT drain 清空，无额外操作），直接 DELETE SQL 会尝试删除 posts（已被 PK-based delete 删除，0 rows，无害）。应继续通过。

**验证**: `cargo test -p rust-ef --test cascade_save_tests` 全部 12 个测试通过。

---

### Step 6：FK DDL ON DELETE 子句（最后做，降低风险）

**文件**: [migration.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration.rs)

#### 6a. `SnapshotColumn` 添加 `fk_on_delete` 字段

在 [line 40-61](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration.rs#L40) 的 `SnapshotColumn` 结构体添加：
```rust
pub fk_on_delete: Option<String>,
```

#### 6b. `SchemaChange::AddForeignKey` 添加 `on_delete` 字段

在 [line 208-218](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration.rs#L208) 的 `AddForeignKey` 变体添加：
```rust
AddForeignKey {
    table: String,
    column: String,
    referenced_table: String,
    referenced_column: String,
    on_delete: Option<String>,  // 新增
},
```

#### 6c. `fk_reference_for_property` 改为按 `fk_column` 匹配 + 返回 ON DELETE

修改 [line 483-503](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration.rs#L483) 的函数：
- 匹配条件从 `nav.foreign_key_field` 改为 `nav.fk_column`（解决可能不匹配的问题）
- 返回类型改为 3-tuple `(Option<String>, Option<String>, Option<String>)`（含 ON DELETE 子句）
- ON DELETE 子句从 `nav.delete_behavior` 解析：Cascade→"CASCADE", SetNull→"SET NULL", Restrict→"RESTRICT", NoAction→"NO ACTION"

#### 6d. `generate_up_sql_inner` 的 `AddForeignKey` 分支追加 ON DELETE

在生成 `ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY ...` SQL 时，若 `on_delete` 为 `Some(clause)`，追加 ` ON DELETE {clause}`。

#### 6e. `columns_structurally_equal` 添加 `fk_on_delete` 比较

在 [line 575-586](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration.rs#L575) 添加 `&& a.fk_on_delete == b.fk_on_delete`。

#### 6f. `snapshot_to_json` / `snapshot_from_json` 序列化新字段

搜索这两函数，添加 `fk_on_delete` 的序列化/反序列化。

#### 6g. 更新 `create_snapshot` / `diff_foreign_keys`

- `create_snapshot`：从 `NavigationMeta.delete_behavior` 解析 ON DELETE 子句，填入 `SnapshotColumn.fk_on_delete`
- `diff_foreign_keys`：`AddForeignKey` 变体填入 `on_delete` 字段

#### 6h. 修复所有 `SnapshotColumn { ... }` / `AddForeignKey { ... }` 构造点

全局搜索这些构造点，补全新字段（`fk_on_delete: None` / `on_delete: None`），包括测试文件。

⚠️ **风险**: 修复 `fk_reference_for_property` 匹配逻辑可能让之前没有 FK 约束的 DDL 突然生成 FK 约束，破坏现有迁移测试。需运行 `cargo test -p rust-ef` 全量验证。

**验证**: `cargo test -p rust-ef` 全量回归通过（除 PostgreSQL 连接测试）。

---

## 实施顺序

1. **Step 4 补全**：添加 `resolve_delete_behavior` → `cargo check -p rust-ef`
2. **Step 5a**：级联 DELETE drain 循环 → `cargo check -p rust-ef`
3. **Step 5b**：直接 DELETE/SET NULL SQL → `cargo check -p rust-ef`
4. **Step 7**：6 个测试 → `cargo test -p rust-ef --test cascade_save_tests`
5. **Step 6**：FK DDL ON DELETE → `cargo test -p rust-ef`（全量回归）

## Verification

1. `cargo check --workspace` — 全 crate 编译通过
2. `cargo test -p rust-ef --test cascade_save_tests` — 12 个测试全通过（6 现有 + 6 新增）
3. `cargo test -p rust-ef` — 全量回归（除 PostgreSQL 连接测试）
4. 重点验证：现有 `cascade_delete_reverse_order` 仍通过

## ⚠️ 需要提醒的问题（已在原计划中列出，这里复述关键项）

1. **直接 DELETE 不应用查询过滤器**：`DELETE WHERE fk = ?` 不含租户隔离等过滤器。多租户场景应先 Include 再删除。
2. **未加载的深层级联**：直接 DELETE 只处理一层。Blog→Post→Comment 全未加载时，只删除 Posts，Comments 需要 DB 级 `ON DELETE CASCADE`（Step 6 的 DDL）或用户手动加载。
3. **SQLite FK 执行**：SQLite 默认 `PRAGMA foreign_keys = OFF`，DB 级 ON DELETE CASCADE 不生效。应用级级联（Steps 1-5）不受影响。
4. **`fk_reference_for_property` 修复风险**：Step 6 可能改变现有 DDL 输出，需验证现有迁移测试不破坏。因此 Step 6 放在最后。
