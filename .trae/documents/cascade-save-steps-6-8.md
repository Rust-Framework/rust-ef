# Cascade Save — 实现计划 (Steps 6-8)

## 摘要

完成 REF ORM 多表级联保存功能。Steps 1-5 已完成（trait 方法、宏代码、dependency_graph、cascade 模块、phase 拆分）。Steps 6-8 待实现：
- **Step 6**: 在 `SetOps<E>` impl 中实现 9 个新的 `ErasedSetOps` trait 方法（当前编译失败，因为 trait 已扩展但 impl 未更新）
- **Step 7**: 重写 `save_changes` 加入级联编排（drain → 拓扑排序 → 有序 INSERT + FK fixup → M2M → UPSERT → UPDATE → DELETE）
- **Step 8**: 创建 `cascade_save_tests.rs`（6 个测试用例）

## 当前状态

### 已完成 (Steps 1-5)
- `entity.rs` (core): `set_foreign_key` + `drain_has_many` trait 方法已添加
- `entity.rs` (macros): 宏生成 `set_fk_arms` + `drain_has_many_arms`，发射 impl 方法
- `dependency_graph.rs`: `build()` / `topological_sort()` / `deletion_order()` 完整
- `cascade.rs`: `DrainedChild` / `FixupLink` / `m2m_insert_sql()` 完整
- `db_context.rs`: `save_one_set` 拆分为 4 个 phase 函数

### 阻塞中 (Step 6 部分)
- `ErasedSetOps` trait 已扩展 9 个新方法签名（`db_context.rs:355-423`）
- `SetOps<E>` impl 块（`db_context.rs:437-486`）**仅实现了原始 4 个方法**，9 个新方法无实现 → **编译失败**

## 需要提醒用户的问题

### 问题 1: 自引用关系的 FK fixup 需要额外 UPDATE
对于自引用关系（如 `Category { parent: BelongsTo<Category>, children: HasMany<Category> }`），drain 提取子节点后，父子在同一 DbSet 中。批量 INSERT 时子节点的 FK=0（父 PK 尚未回填）。INSERT + 回填后，需要额外 `UPDATE child SET fk = parent_pk WHERE pk = child_pk` 来修正 FK。

EFCore 内部也这样处理（先 INSERT NULL，再 UPDATE FK）。对跨类型关系无此问题——拓扑排序保证父类型先 INSERT，子类型 FK 在内存中设置后再 INSERT。

### 问题 2: 级联子类型必须预先注册 DbSet
如果 `Blog` 有 `HasMany<Post>`，用户必须在 `save_changes` 前调用 `ctx.set::<Post>()`。否则 drain 产生的子实体无法添加到目标 DbSet（无类型擦除的工厂机制可自动创建 `DbSet<T>`）。如果未注册，返回明确错误信息而非静默丢弃。

### 问题 3: M2M 已有子实体的处理
M2M 场景中，HasMany 里的子实体可能是新的（PK=0）或已有的（PK>0）。`add_cascade_child` 内部检查 PK：
- PK=0 → `add()` 为 Added（INSERT 子实体 + INSERT join row）
- PK>0 → `attach()` 为 Unchanged（仅 INSERT join row，不重复 INSERT 子实体）

一对多场景同样适用此逻辑：已有子实体不会被重复 INSERT。

---

## Step 6: 实现 9 个 `ErasedSetOps` 方法

**文件**: `crates/core/src/db_context.rs`
**位置**: `impl<E> ErasedSetOps for SetOps<E>` 块（当前 line 437-486）

在现有 4 个方法（`save`, `detect_changes`, `accept_all_changes`, `collect_entries`）之后，添加 9 个新方法实现：

### 6.1 `drain_cascade_children`
```rust
fn drain_cascade_children(
    &self,
    raw_set: &mut (dyn Any + Send + Sync),
    meta: &EntityTypeMeta,
) -> Vec<DrainedChild> {
    let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (entry_idx, entry) in db_set.entries.iter_mut().enumerate() {
        // 只 drain Added 且非 upsert 的主实体
        if entry.state != EntityState::Added || entry.is_upsert {
            continue;
        }
        for nav in &meta.navigations {
            if !matches!(nav.kind, NavigationKind::HasMany | NavigationKind::ManyToMany) {
                continue;
            }
            if let Some(items) = entry.entity.drain_has_many(nav.field_name.as_ref()) {
                for item in items {
                    result.push(DrainedChild {
                        parent_type_id: TypeId::of::<E>(),
                        parent_entry_idx: entry_idx,
                        child: item,
                        child_type_id: nav.related_type_id,
                        fk_target_type_id: TypeId::of::<E>(),
                        through_table: nav.through_table.as_ref().map(|s| s.to_string()),
                        through_parent_fk_col: nav.through_parent_fk.as_ref().map(|s| s.to_string()),
                        through_child_fk_col: nav.through_related_fk.as_ref().map(|s| s.to_string()),
                    });
                }
            }
        }
    }
    result
}
```

### 6.2 `add_cascade_child`
```rust
fn add_cascade_child(
    &self,
    raw_set: &mut (dyn Any + Send + Sync),
    child: Box<dyn Any + Send + Sync>,
) -> Option<usize> {
    let db_set = raw_set.downcast_mut::<DbSet<E>>()?;
    let child = child.downcast::<E>().ok()?;
    // 检查 PK：PK>0 表示已有实体，attach 为 Unchanged；PK=0 表示新实体，add 为 Added
    let pk: i64 = child.key_values().into_values().next()
        .and_then(|v| v.clone().try_into().ok())
        .unwrap_or(0);
    if pk > 0 {
        db_set.attach((*child).clone());
    } else {
        db_set.add((*child).clone());
    }
    Some(db_set.entries.len() - 1)
}
```

**注意**: `child` 是 `Box<E>`，downcast 后得到 `Box<E>`，需要解引用 clone 或移动。实际上 `downcast::<E>()` 返回 `Result<Box<E>, Box<dyn Any>>`，所以 `child` 是 `Box<E>`。可以用 `*child` 移动。

修正：
```rust
let child = child.downcast::<E>().ok()?;
let pk: i64 = child.key_values().into_values().next()
    .and_then(|v| v.try_into().ok())
    .unwrap_or(0);
let child = *child;  // 解 box
if pk > 0 {
    db_set.attach(child);
} else {
    db_set.add(child);
}
Some(db_set.entries.len() - 1)
```

### 6.3 `entry_count`
```rust
fn entry_count(&self, raw_set: &(dyn Any + Send + Sync)) -> usize {
    raw_set.downcast_ref::<DbSet<E>>().map(|s| s.entries.len()).unwrap_or(0)
}
```

### 6.4 `get_pk_at`
```rust
fn get_pk_at(&self, raw_set: &(dyn Any + Send + Sync), idx: usize) -> Option<i64> {
    let db_set = raw_set.downcast_ref::<DbSet<E>>()?;
    let entry = db_set.entries.get(idx)?;
    entry.entity.key_values().into_values().next()
        .and_then(|v| v.try_into().ok())
}
```

### 6.5 `set_fk_at`
```rust
fn set_fk_at(
    &self,
    raw_set: &mut (dyn Any + Send + Sync),
    idx: usize,
    target_type: TypeId,
    key: i64,
) {
    if let Some(db_set) = raw_set.downcast_mut::<DbSet<E>>() {
        if let Some(entry) = db_set.entries.get_mut(idx) {
            entry.entity.set_foreign_key(target_type, key);
        }
    }
}
```

### 6.6 `insert_added`
```rust
async fn insert_added(
    &self,
    conn: &mut (dyn IAsyncConnection + Send),
    provider: &dyn IDatabaseProvider,
    raw_set: &mut (dyn Any + Send + Sync),
    meta: &EntityTypeMeta,
) -> EFResult<usize> {
    let db_set = raw_set.downcast_mut::<DbSet<E>>().expect("SetOps type mismatch");
    insert_added_phase(conn, provider, db_set, meta).await
}
```

### 6.7 `upsert_added`
```rust
async fn upsert_added(
    &self,
    conn: &mut (dyn IAsyncConnection + Send),
    provider: &dyn IDatabaseProvider,
    raw_set: &mut (dyn Any + Send + Sync),
    meta: &EntityTypeMeta,
) -> EFResult<usize> {
    let db_set = raw_set.downcast_mut::<DbSet<E>>().expect("SetOps type mismatch");
    upsert_added_phase(conn, provider, db_set, meta).await
}
```

### 6.8 `update_modified`
```rust
async fn update_modified(
    &self,
    conn: &mut (dyn IAsyncConnection + Send),
    provider: &dyn IDatabaseProvider,
    raw_set: &mut (dyn Any + Send + Sync),
    meta: &EntityTypeMeta,
) -> EFResult<usize> {
    let db_set = raw_set.downcast_mut::<DbSet<E>>().expect("SetOps type mismatch");
    let query_filter = db_set.query_filter().cloned();
    update_modified_phase(conn, provider, db_set, meta, query_filter.as_ref()).await
}
```

### 6.9 `delete_deleted`
```rust
async fn delete_deleted(
    &self,
    conn: &mut (dyn IAsyncConnection + Send),
    provider: &dyn IDatabaseProvider,
    raw_set: &mut (dyn Any + Send + Sync),
    meta: &EntityTypeMeta,
) -> EFResult<usize> {
    let db_set = raw_set.downcast_mut::<DbSet<E>>().expect("SetOps type mismatch");
    let query_filter = db_set.query_filter().cloned();
    delete_deleted_phase(conn, provider, db_set, meta, query_filter.as_ref()).await
}
```

**验证**: `cargo check -p rust-ef` 通过编译。

---

## Step 7: 重写 `save_changes` 加入级联编排

**文件**: `crates/core/src/db_context.rs`
**位置**: `save_changes` 方法（当前 line 764-879）

### 新流程

```
1. detect_changes（不变）
2. build configured_metas（不变）
3. interceptor on_saving（不变）
4. 获取事务连接（不变）
5. 【新】级联 drain 循环
6. 【新】拓扑排序
7. 【新】INSERT 阶段（按拓扑序）+ FK fixup + 自引用 UPDATE
8. 【新】M2M join row 插入
9. UPSERT 阶段（按拓扑序）
10. UPDATE 阶段（按拓扑序）
11. 【新】DELETE 阶段（按逆拓扑序）
12. commit（不变）
13. accept_all_changes（不变）
14. interceptor on_saved（不变）
```

### 7.1 级联 drain 循环（步骤 5）

迭代 drain 直到不动点：

```rust
let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
let mut fixup_links: Vec<FixupLink> = Vec::new();

loop {
    let mut all_drained: Vec<DrainedChild> = Vec::new();
    for type_id in &type_ids {
        let saver = self.savers.get(type_id).expect("saver not registered");
        let set = self.sets.get_mut(type_id).unwrap();
        let meta = configured_metas.get(type_id)
            .or_else(|| self.entity_metas.get(type_id))
            .expect("meta not found");
        let drained = saver.drain_cascade_children(set.as_mut(), meta);
        all_drained.extend(drained);
    }
    if all_drained.is_empty() {
        break;
    }
    for child in all_drained {
        // 查找子类型的 saver 和 set
        let child_saver = self.savers.get(&child.child_type_id)
            .ok_or_else(|| EFError::configuration(format!(
                "Cannot cascade-save child type {:?}: no DbSet registered. \
                 Call ctx.set::<ChildType>() before save_changes.",
                child.child_type_id
            )))?;
        let child_set = self.sets.get_mut(&child.child_type_id)
            .expect("set not found for registered saver");
        if let Some(child_idx) = child_saver.add_cascade_child(child_set.as_mut(), child.child) {
            // 查找或创建 FixupLink（按 parent+navigation 分组）
            if let Some(link) = fixup_links.iter_mut().find(|l| 
                l.parent_type_id == child.parent_type_id 
                && l.parent_entry_idx == child.parent_entry_idx
                && l.child_type_id == child.child_type_id
                && l.through_table == child.through_table
            ) {
                link.child_entry_indices.push(child_idx);
            } else {
                fixup_links.push(FixupLink {
                    parent_type_id: child.parent_type_id,
                    parent_entry_idx: child.parent_entry_idx,
                    child_type_id: child.child_type_id,
                    child_entry_indices: vec![child_idx],
                    fk_target_type_id: child.fk_target_type_id,
                    through_table: child.through_table,
                    through_parent_fk_col: child.through_parent_fk_col,
                    through_child_fk_col: child.through_child_fk_col,
                });
            }
        }
    }
}
```

### 7.2 拓扑排序（步骤 6）

```rust
let graph = DependencyGraph::build(&configured_metas);
let insert_order = graph.topological_sort();
let delete_order = graph.deletion_order();
```

### 7.3 INSERT 阶段 + FK fixup（步骤 7）

```rust
let mut total_added = 0usize;
let mut total_updated = 0usize;
let mut total_deleted = 0usize;
let conn_ref: &mut dyn IAsyncConnection = match &mut txn {
    TxnSource::Ambient(t) => t.connection(),
    TxnSource::Managed(c) => c.as_mut(),
};

// INSERT 阶段：按拓扑序
for type_id in &insert_order {
    let saver = self.savers.get(type_id).expect("saver not registered");
    let set = self.sets.get_mut(type_id).unwrap();
    let meta = configured_metas.get(type_id)
        .or_else(|| self.entity_metas.get(type_id))
        .expect("meta not found");
    
    let inserted = saver.insert_added(conn_ref, &*self.provider, set.as_mut(), meta).await?;
    total_added += inserted;
    
    // FK fixup：处理 parent_type == type_id 的 FixupLink
    let links_for_this_type: Vec<&FixupLink> = fixup_links.iter()
        .filter(|l| l.parent_type_id == *type_id && l.through_table.is_none())
        .collect();
    
    for link in links_for_this_type {
        // 读取父 PK
        let parent_set = self.sets.get(&link.parent_type_id).unwrap();
        let parent_pk = saver.get_pk_at(parent_set.as_ref(), link.parent_entry_idx);
        if let Some(pk) = parent_pk {
            let child_saver = self.savers.get(&link.child_type_id).unwrap();
            let child_set = self.sets.get_mut(&link.child_type_id).unwrap();
            for &child_idx in &link.child_entry_indices {
                child_saver.set_fk_at(
                    child_set.as_mut(),
                    child_idx,
                    link.fk_target_type_id,
                    pk,
                );
                // 自引用：子实体已 INSERT（FK=0），需要 UPDATE
                if link.child_type_id == link.parent_type_id {
                    // 生成 UPDATE SQL
                    let child_pk = child_saver.get_pk_at(child_set.as_ref(), child_idx);
                    if let Some(child_pk) = child_pk {
                        let child_meta = configured_metas.get(&link.child_type_id)
                            .or_else(|| self.entity_metas.get(&link.child_type_id))
                            .unwrap();
                        // 找到 FK 列名
                        let fk_col = child_meta.navigations.iter()
                            .find(|n| n.kind == NavigationKind::BelongsTo 
                                && n.related_type_id == link.fk_target_type_id)
                            .and_then(|n| n.fk_column.as_ref())
                            .or_else(|| {
                                // 或者从 FK scalar field 找
                                child_meta.properties.iter()
                                    .find(|p| p.is_foreign_key)
                                    .map(|p| p.column_name.as_ref())
                            });
                        if let Some(fk_col) = fk_col {
                            let sql = format!(
                                "UPDATE {} SET {} = ? WHERE {} = ?",
                                child_meta.table_name, fk_col,
                                child_meta.properties.iter()
                                    .find(|p| p.is_primary_key)
                                    .map(|p| p.column_name.as_ref())
                                    .unwrap_or("id")
                            );
                            conn_ref.execute(&sql, &[
                                DbValue::from(pk),
                                DbValue::from(child_pk),
                            ]).await?;
                        }
                    }
                }
            }
        }
    }
}
```

**注意**: 上面的自引用 UPDATE 逻辑较复杂，需要从 meta 中查找 FK 列名和 PK 列名。如果 FK 列名查找失败（macro bug: `extract_foreign_key_field_name` 返回 None），需要回退到 scalar field 的 `is_foreign_key` 标志。

### 7.4 M2M join row 插入（步骤 8）

```rust
// M2M join row 插入：所有实体 INSERT 完成后
for link in &fixup_links {
    if link.through_table.is_none() {
        continue;  // 跳过一对多
    }
    let table = link.through_table.as_ref().unwrap();
    let parent_col = link.through_parent_fk_col.as_ref().unwrap();
    let child_col = link.through_child_fk_col.as_ref().unwrap();
    
    // 读取父 PK
    let parent_set = self.sets.get(&link.parent_type_id).unwrap();
    let parent_saver = self.savers.get(&link.parent_type_id).unwrap();
    let parent_pk = parent_saver.get_pk_at(parent_set.as_ref(), link.parent_entry_idx);
    
    if let Some(parent_pk) = parent_pk {
        let child_set = self.sets.get(&link.child_type_id).unwrap();
        let child_saver = self.savers.get(&link.child_type_id).unwrap();
        
        // 收集所有子 PK
        let mut child_pks: Vec<i64> = Vec::new();
        for &child_idx in &link.child_entry_indices {
            if let Some(child_pk) = child_saver.get_pk_at(child_set.as_ref(), child_idx) {
                child_pks.push(child_pk);
            }
        }
        
        if !child_pks.is_empty() {
            let sql = cascade::m2m_insert_sql(table, parent_col, child_col, child_pks.len());
            let mut params: Vec<DbValue> = Vec::with_capacity(child_pks.len() * 2);
            for child_pk in &child_pks {
                params.push(DbValue::from(parent_pk));
                params.push(DbValue::from(*child_pk));
            }
            conn_ref.execute(&sql, &params).await?;
            total_added += child_pks.len();
        }
    }
}
```

### 7.5 UPSERT / UPDATE / DELETE 阶段（步骤 9-11）

```rust
// UPSERT 阶段：按拓扑序
for type_id in &insert_order {
    let saver = self.savers.get(type_id).unwrap();
    let set = self.sets.get_mut(type_id).unwrap();
    let meta = configured_metas.get(type_id).or_else(|| self.entity_metas.get(type_id)).unwrap();
    total_added += saver.upsert_added(conn_ref, &*self.provider, set.as_mut(), meta).await?;
}

// UPDATE 阶段：按拓扑序
for type_id in &insert_order {
    let saver = self.savers.get(type_id).unwrap();
    let set = self.sets.get_mut(type_id).unwrap();
    let meta = configured_metas.get(type_id).or_else(|| self.entity_metas.get(type_id)).unwrap();
    total_updated += saver.update_modified(conn_ref, &*self.provider, set.as_mut(), meta).await?;
}

// DELETE 阶段：按逆拓扑序（依赖方先删）
for type_id in &delete_order {
    let saver = self.savers.get(type_id).unwrap();
    let set = self.sets.get_mut(type_id).unwrap();
    let meta = configured_metas.get(type_id).or_else(|| self.entity_metas.get(type_id)).unwrap();
    total_deleted += saver.delete_deleted(conn_ref, &*self.provider, set.as_mut(), meta).await?;
}
```

### 7.6 错误处理

保留原有的 rollback 逻辑：任何阶段失败时，回滚事务（Managed）或恢复 ambient（Ambient），调用 `on_save_failed`。

### 7.7 保留 `save_one_set` 公共函数

`save_one_set` 及 4 个 phase 函数保持 public，供 `SetOps<E>::save` 和 `SetOps<E>::insert_added` 等方法调用。`save_changes` 不再调用 `saver.save()`，而是直接调用 phase 方法。

**验证**: `cargo check -p rust-ef` 通过编译。

---

## Step 8: 创建级联保存测试

**文件**: `crates/core/tests/cascade_save_tests.rs`（新建）

### 测试实体定义

```rust
// 一对多
#[derive(Debug, Clone, EntityType)]
#[table("cascade_blogs")]
struct CascadeBlog {
    #[primary_key]
    #[auto_increment]
    blog_id: i32,
    #[required]
    url: String,
    #[navigation]
    posts: HasMany<CascadePost>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_posts")]
struct CascadePost {
    #[primary_key]
    #[auto_increment]
    post_id: i32,
    #[required]
    title: String,
    #[foreign_key(CascadeBlog)]
    blog_id: i32,
    #[navigation]
    blog: BelongsTo<CascadeBlog>,
}

// 自引用
#[derive(Debug, Clone, EntityType)]
#[table("cascade_categories")]
struct CascadeCategory {
    #[primary_key]
    #[auto_increment]
    category_id: i32,
    #[required]
    name: String,
    #[foreign_key(CascadeCategory)]
    parent_id: i32,
    #[navigation]
    children: HasMany<CascadeCategory>,
}

// M2M
#[derive(Debug, Clone, EntityType)]
#[table("cascade_students")]
struct CascadeStudent {
    #[primary_key]
    #[auto_increment]
    student_id: i32,
    #[required]
    name: String,
    #[navigation]
    courses: HasMany<CascadeCourse, CascadeEnrollment>,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_courses")]
struct CascadeCourse {
    #[primary_key]
    #[auto_increment]
    course_id: i32,
    #[required]
    title: String,
}

#[derive(Debug, Clone, EntityType)]
#[table("cascade_enrollments")]
struct CascadeEnrollment {
    #[primary_key]
    #[auto_increment]
    enrollment_id: i32,
    #[foreign_key(CascadeStudent)]
    student_id: i32,
    #[foreign_key(CascadeCourse)]
    course_id: i32,
}
```

### 测试用例

1. **`cascade_insert_blog_with_posts`**: 创建 Blog 含 2 个 Posts，save_changes 后验证 Blog PK 回填、Post FK 正确设置、DB 中数据一致。

2. **`cascade_insert_self_referential_tree`**: 创建 Category 含子节点，save_changes 后验证子节点 parent_id 正确（通过 UPDATE fixup）。

3. **`cascade_insert_many_to_many`**: 创建 Student 含 2 个新 Courses，save_changes 后验证 Student/Course PK 回填、enrollment join row 存在。

4. **`cascade_update_ordering`**: 修改 Blog 和 Post，save_changes 后验证 UPDATE 顺序正确（拓扑序）。

5. **`cascade_delete_reverse_order`**: 删除 Blog 和其 Posts，save_changes 后验证 DELETE 顺序正确（逆拓扑序，Posts 先删）。

6. **`cascade_empty_has_many_noop`**: 创建 Blog 含空 HasMany，save_changes 后验证正常工作（无 drain、无错误）。

### 测试模式

遵循 `m2m_tests.rs` 和 `navigation_tests.rs` 的模式：
- SQLite in-memory (`use_sqlite_in_memory()`)
- `ctx.set::<T>()` 注册所有实体类型
- `ctx.ensure_created().await` 创建表
- `ctx.save_changes().await` 执行级联保存
- `ctx.set::<T>().query().to_list().await` 验证结果

**验证**: `cargo test -p rust-ef --test cascade_save_tests` 全部通过。

---

## 验证步骤

1. `cargo check -p rust-ef` — Step 6 完成后编译通过
2. `cargo check -p rust-ef` — Step 7 完成后编译通过
3. `cargo test -p rust-ef --test cascade_save_tests` — Step 8 完成后测试通过
4. `cargo test -p rust-ef` — 全量回归测试通过
5. `cargo test --workspace` — 全 workspace 测试通过

## 文件变更清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `crates/core/src/db_context.rs` | 修改 | Step 6: SetOps<E> impl 添加 9 方法；Step 7: 重写 save_changes |
| `crates/core/tests/cascade_save_tests.rs` | 新建 | Step 8: 6 个级联保存测试 |

## 假设与决策

| 假设 | 决策 |
|------|------|
| 级联子类型必须预先注册 | 未注册时返回错误，提示用户调用 `ctx.set::<ChildType>()` |
| M2M 子实体可能是新的或已有的 | `add_cascade_child` 内部检查 PK：PK>0 attach，PK=0 add |
| 自引用关系 FK fixup | INSERT 后额外 UPDATE 修正 FK |
| 跨类型关系 FK fixup | 拓扑序保证父先 INSERT，子 FK 在内存中设置后再 INSERT |
| drain 只处理 Added 主实体 | Modified/Deleted 主实体不 drain（其 HasMany 子实体不级联） |
| HasMany 方向 only | BelongsTo/HasOne 不 drain（是引用，非拥有的子实体） |
| 复合主键 | `get_pk_at` 取第一个 PK 值，复合主键场景为已知限制 |
