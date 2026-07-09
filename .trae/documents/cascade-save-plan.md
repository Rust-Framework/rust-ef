# 多表关联插入、更新实现计划

## 1. 回填确认（Part 1 — 已验证）

PK 回填**仅在** `is_auto_increment && is_primary_key` 时触发：

- [change_executor.rs:60-63](file:///d:/GitCode/RF/rust-ef/crates/core/src/change_executor.rs#L60-L63) — `auto_inc_pk = scalar_props.iter().find(|p| p.is_auto_increment && p.is_primary_key)`
- 三条回填路径：PostgreSQL `RETURNING *`、SQLite/MySQL `last_insert_id()`、回退路径（key=0，no-op）
- 当 `auto_inc_pk` 为 `None` 时，`else` 分支回填 `0`，`set_auto_increment_key` 忽略

**序列（Sequence）**：REF 无独立序列概念。PostgreSQL `SERIAL`/`BIGSERIAL`/`SMALLSERIAL` 由 `#[auto_increment]` 映射（[postgres type_mapping.rs:36-39](file:///d:/GitCode/RF/rust-ef/crates/postgres/src/type_mapping.rs#L36-L39)，[migration.rs:87-94](file:///d:/GitCode/RF/rust-ef/crates/core/src/migration.rs#L87-L94)）。序列即自增，由同一回填机制覆盖。

---

## 2. 当前架构分析

### 2.1 save_changes 编排（[db_context.rs:690-805](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L690-L805)）
- 按 `self.sets.keys()` 的 HashMap 迭代顺序逐类型保存，**无依赖排序**
- 每个类型调用 `saver.save()` → `save_one_set()`
- 事务：Managed（自管 begin/commit）或 Ambient（`use_transaction`）

### 2.2 save_one_set 三阶段（[db_context.rs:863-947](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L863-L947)）
- Phase 1a: INSERT Added（非 upsert）→ `backfill_added_keys`
- Phase 1b: UPSERT Added（`is_upsert = true`）
- Phase 2: UPDATE Modified（partial update via `modified_properties`）
- Phase 3: DELETE Deleted
- 借用检查器约束：每个阶段独立 block，阶段间释放 `db_set` 借用

### 2.3 类型擦除分发（[db_context.rs:331-412](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L331-L412)）
- `ErasedSetOps` trait：`save`、`detect_changes`、`accept_all_changes`、`collect_entries`
- `SetOps<E>` 泛型实现，downcast `Box<dyn Any>` → `DbSet<E>`

### 2.4 导航容器（[relations.rs](file:///d:/GitCode/RF/rust-ef/crates/core/src/relations.rs)）
- `HasMany<T, Join>`：`items: Vec<T>` 按值持有，`items_mut() -> &mut Vec<T>`
- `BelongsTo<T>`：`inner: Option<Box<T>>` 按值持有，`take() -> Option<T>`
- Clone 刻意丢弃内部数据
- **关键**：`blog.posts` 中的 Post 实体**不在** `ctx.set::<Post>()` 中

### 2.5 导航元数据（[metadata.rs:147-184](file:///d:/GitCode/RF/rust-ef/crates/core/src/metadata.rs#L147-L184)）
- `NavigationMeta`：`kind`、`related_type_id`、`fk_column`、`referenced_key_column`、`field_name`
- M2M：`through_type_id`、`through_table`、`through_parent_fk`、`through_related_fk`
- `foreign_key_field` **始终为 None**（[entity.rs:789](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L789) — 已知宏 bug，本计划不修复此字段，改用 `IPropertySetter` 机制）

### 2.6 宏的 FK 声明（[entity.rs:385-402](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L385-L402)）
- `#[foreign_key(Blog)] pub blog_id: i32` → 生成 `FK_Blog: &str = "blog_id"` 常量
- HasMany 导航元数据的 `fk_column` = 子实体的 `FK_<Parent>` 常量值
- **可用于生成 `set_foreign_key(target_type, key)` 的 match arm**

---

## 3. 引入的问题与权衡（必须提醒用户）

### 3.1 破坏性抽取（Destruction）
save_changes 后 `HasMany` 容器变空——实体所有权从导航容器转移到 DbSet。用户需重新查询（Include）来恢复导航数据。这是 Rust 所有权模型的必然结果（EFCore 用引用，非破坏性）。

### 3.2 M2M 联表行通过直接 SQL 插入
`HasMany<T, Join>` 容器存储的是 `T`（关联实体），**不存储** Join 实体。M2M 级联时：
- 抽取 T 实体 → 加入 T 的 DbSet → INSERT
- 父子 PK 回填后，用元数据直接执行 `INSERT INTO through_table (parent_fk, child_fk) VALUES (?, ?)`
- **联表行不受变更追踪管理**——不能通过 change tracker 更新/删除联表行（仅插入）。未来需扩展。

### 3.3 BelongsTo 反向级联不包含
从 `BelongsTo<T>` 抽取父实体过于破坏性（子实体失去父引用）。本计划仅实现 HasMany 方向（父→子）级联。如果子实体的 `BelongsTo<Parent>` 中有一个新 Parent，用户需显式将 Parent 加入其 DbSet。

### 3.4 仅从 Added 主体抽取
只从 **Added**（新建）主体的 HasMany 抽取子实体。不抽取 Unchanged/Modified 主体的导航子实体（它们已通过 Include/load 加载，已在 DbSet 中追踪）。如果用户给已存在的父实体添加新子实体，需显式将子实体加入 DbSet。

### 3.5 删除级联（DeleteBehavior）推迟
用户要求"插入、更新"。本计划实现：
- INSERT 级联（新父+新子，FK fixup）
- UPDATE/DELETE 按拓扑序保存（父先于子更新，子先于父删除）
**不实现** `DeleteBehavior::Cascade/SetNull/Restrict` 驱动的自动级联删除——这是独立的大功能。

### 3.6 PK 变更传播不包含
如果 Modified 主体的 PK 被修改（罕见），子实体的 FK 不会自动更新。这是边界场景，推迟。

### 3.7 用户必须预注册所有级联类型
Rust 编译时约束：用户必须在 save_changes 前调用 `ctx.set::<Post>()`（至少一次）以确保 `SetOps<Post>` 注册。无法在运行时通过 TypeId 泛型创建 DbSet。这类似 EFCore 要求在 Context 中声明 `DbSet<Post>` 属性。

### 3.8 性能
迭代抽取循环按深度层级运行（自引用树安全，无递归栈溢出）。无导航的实体类型抽取为空操作（最小开销）。

---

## 4. 实现步骤

### Step 1: `set_foreign_key` 方法（IGetKeyValues 扩展）

**文件**: `crates/core/src/entity.rs`

在 `IGetKeyValues` trait 添加 `set_foreign_key` 方法（与 `set_auto_increment_key` 一致的模式）：

```rust
pub trait IGetKeyValues: IEntityType {
    fn key_values(&self) -> HashMap<String, DbValue>;
    fn set_auto_increment_key(&mut self, _key: i64) {}
    /// 设置指向 target_type 的外键字段为 key。
    /// 由 #[derive(EntityType)] 为每个 #[foreign_key(Target)] 字段生成。
    fn set_foreign_key(&mut self, _target_type: TypeId, _key: i64) {}
}
```

需在 `entity.rs` 顶部添加 `use std::any::TypeId`（如已有则跳过）。

**文件**: `crates/macros/src/entity.rs`

在宏展开中，为每个 `#[foreign_key(Target)]` 标量字段生成 `set_foreign_key` match arm。复用现有的 `fk_const_decls` 循环中已提取的 `target`/`target_ident`：

```rust
// 在 IGetKeyValues impl 块中，set_auto_increment_key 之后：
fn set_foreign_key(&mut self, target_type: std::any::TypeId, key: i64) {
    #( #set_fk_arms )*
    let _ = (target_type, key);
}
```

其中 `set_fk_arms` 在 FK 字段处理循环（[entity.rs:385-402](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L385-L402)）中收集：
```rust
set_fk_arms.push(quote! {
    if target_type == std::any::TypeId::of::<#target_ident>() {
        self.#field_name = key as _;
    }
});
```

### Step 2: `drain_has_many` 方法（INavigationSetter 扩展）

**文件**: `crates/core/src/entity.rs`

在 `INavigationSetter` trait 添加 `drain_has_many` 方法：

```rust
#[async_trait::async_trait]
pub trait INavigationSetter: IEntityType {
    // 现有方法不变...

    /// 从 HasMany 导航字段抽取所有项，返回类型擦除的 boxed 值。
    /// 抽取后容器为空。用于级联保存时从主体提取 Added 子实体。
    /// 返回 None 表示该字段不是 HasMany 导航或容器为空。
    fn drain_has_many(
        &mut self,
        field: &str,
    ) -> Option<Vec<Box<dyn std::any::Any + Send + Sync>>> {
        let _ = field;
        None
    }
}
```

**文件**: `crates/macros/src/entity.rs`

在 HasMany 分支（[entity.rs:199-243](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L199-L243)）中，为每个 HasMany 字段生成 `drain_has_many` arm。新增 `drain_has_many_arms: Vec<TokenStream>`（在顶部与其他 arms 一起声明）：

```rust
// HasMany 分支中：
drain_has_many_arms.push(quote! {
    if field == #field_name_str {
        let items = std::mem::take(self.#field_name.items_mut());
        if items.is_empty() {
            return None;
        }
        return Some(items.into_iter()
            .map(|item| Box::new(item) as Box<dyn std::any::Any + Send + Sync>)
            .collect());
    }
});
```

在 `INavigationSetter` impl 块中（[entity.rs:601-620](file:///d:/GitCode/RF/rust-ef/crates/macros/src/entity.rs#L601-L620)）添加：
```rust
fn drain_has_many(
    &mut self,
    field: &str,
) -> Option<Vec<Box<dyn std::any::Any + Send + Sync>>> {
    #( #drain_has_many_arms )*
    None
}
```

### Step 3: `dependency_graph.rs` 模块（拓扑排序）

**文件**: `crates/core/src/dependency_graph.rs`（新建）

```rust
//! 依赖图与拓扑排序——确定实体类型的保存顺序。
//! 主体先于依赖者（FK 在依赖者一侧）。删除顺序为反向。

use std::any::TypeId;
use std::collections::{HashMap, HashSet, VecDeque};
use crate::metadata::EntityTypeMeta;

pub struct DependencyGraph {
    /// type_id → 其依赖的类型（主体的 type_id）
    edges: HashMap<TypeId, Vec<TypeId>>,
    nodes: Vec<TypeId>,
}

impl DependencyGraph {
    /// 从所有实体元数据构建。边来自 HasMany 导航：
    /// related_type_id（子）依赖 type_id（父）。
    pub fn build(metas: &HashMap<TypeId, EntityTypeMeta>) -> Self {
        let mut edges: HashMap<TypeId, Vec<TypeId>> = HashMap::new();
        let mut nodes: Vec<TypeId> = Vec::new();
        for (type_id, meta) in metas {
            nodes.push(*type_id);
            for nav in &meta.navigations {
                if matches!(nav.kind, crate::metadata::NavigationKind::HasMany
                    | crate::metadata::NavigationKind::ManyToMany)
                {
                    // child (related_type_id) depends on parent (type_id)
                    edges.entry(nav.related_type_id)
                        .or_default()
                        .push(*type_id);
                }
            }
        }
        Self { edges, nodes }
    }

    /// Kahn 算法拓扑排序：主体在前，依赖者在后。
    /// 自引用（type_id 依赖自身）不影响类型级排序——实例级顺序由抽取+fixup 保证。
    pub fn topological_sort(&self) -> Vec<TypeId> {
        let mut in_degree: HashMap<TypeId, usize> = HashMap::new();
        for node in &self.nodes {
            in_degree.entry(*node).or_insert(0);
        }
        for deps in self.edges.values() {
            for dep in deps {
                // dep 依赖 parent，但这里我们计算"被依赖"的入度
                // 实际上 edges: child -> [parents]，所以 child 的入度 += 1
            }
        }
        // 重新计算：in_degree[child] = number of parents
        for (child, parents) in &self.edges {
            // 自引用排除：child == parent 时不计入（同类型内由 fixup 处理）
            let count = parents.iter().filter(|p| **p != *child).count();
            *in_degree.entry(*child).or_insert(0) += count;
        }
        let mut queue: VecDeque<TypeId> = in_degree.iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&k, _)| k)
            .collect();
        let mut result = Vec::new();
        while let Some(node) = queue.pop_front() {
            result.push(node);
            // 找到所有依赖 node 的 child，减少其入度
            for (child, parents) in &self.edges {
                if parents.iter().any(|p| *p == node && *p != *child) {
                    let deg = in_degree.entry(*child).or_insert(0);
                    if *deg > 0 { *deg -= 1; }
                    if *deg == 0 && !result.contains(child) && !queue.contains(child) {
                        queue.push_back(*child);
                    }
                }
            }
        }
        // 处理未入队的节点（环或自引用）
        for node in &self.nodes {
            if !result.contains(node) {
                result.push(*node);
            }
        }
        result
    }

    /// 删除顺序：拓扑逆序（子先于父）。
    pub fn deletion_order(&self) -> Vec<TypeId> {
        let mut order = self.topological_sort();
        order.reverse();
        order
    }
}
```

**文件**: `crates/core/src/lib.rs` — 添加 `pub mod dependency_graph;`

### Step 4: `cascade.rs` 模块（抽取 + Fixup + M2M 联表）

**文件**: `crates/core/src/cascade.rs`（新建）

```rust
//! 级联保存：从主体 HasMany 抽取子实体、FK fixup、M2M 联表插入。

use crate::entity::{EntityState, IEntityType};
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, IAsyncConnection, IDatabaseProvider};
use crate::error::EFResult;
use std::any::{Any, TypeId};
use std::collections::HashMap;

/// 抽取的子实体及其父关联信息。
pub struct DrainedChild {
    pub parent_type_id: TypeId,
    pub parent_entry_idx: usize,
    pub child: Box<dyn Any + Send + Sync>,
    pub child_type_id: TypeId,
    /// FK fixup 目标类型（set_foreign_key 的 target_type 参数）
    pub fk_target_type_id: TypeId,
    /// M2M: through_table 名称（None 表示一对多）
    pub through_table: Option<String>,
    pub through_parent_fk_col: Option<String>,
    pub through_child_fk_col: Option<String>,
}

/// 记录父→子关联，用于 INSERT 后 FK fixup。
pub struct FixupLink {
    pub parent_type_id: TypeId,
    pub parent_entry_idx: usize,
    pub child_type_id: TypeId,
    pub child_entry_indices: Vec<usize>,
    pub fk_target_type_id: TypeId,
    /// M2M 联表信息
    pub through_table: Option<String>,
    pub through_parent_fk_col: Option<String>,
    pub through_child_fk_col: Option<String>,
}

/// M2M 联表行插入 SQL 生成。
pub fn m2m_insert_sql(
    table: &str,
    parent_col: &str,
    child_col: &str,
    row_count: usize,
) -> String {
    let placeholders: Vec<String> = (0..row_count)
        .map(|i| format!("(?, ?)") )
        .collect();
    format!(
        "INSERT INTO {} ({}, {}) VALUES {}",
        table, parent_col, child_col, placeholders.join(", ")
    )
}
```

**文件**: `crates/core/src/lib.rs` — 添加 `pub mod cascade;`

### Step 5: 拆分 `save_one_set` 为阶段函数

**文件**: `crates/core/src/db_context.rs`

将 `save_one_set` 的 3 个 block 提取为 4 个独立函数（保持现有逻辑不变）：

```rust
pub async fn insert_added_phase<E>(
    conn: &mut dyn IAsyncConnection,
    provider: &dyn IDatabaseProvider,
    db_set: &mut DbSet<E>,
    meta: &EntityTypeMeta,
) -> EFResult<usize>
where E: IEntityType + IEntitySnapshot + IGetKeyValues
{
    // Phase 1a 逻辑（非 upsert 的 Added）
    // 返回插入行数
}

pub async fn upsert_added_phase<E>(...) -> EFResult<usize> { /* Phase 1b */ }
pub async fn update_modified_phase<E>(...) -> EFResult<usize> { /* Phase 2 */ }
pub async fn delete_deleted_phase<E>(...) -> EFResult<usize> { /* Phase 3 */ }
```

`save_one_set` 保留，内部调用这 4 个函数（保持向后兼容）。

### Step 6: 扩展 `ErasedSetOps` 与 `SetOps<E>`

**文件**: `crates/core/src/db_context.rs`

在 `ErasedSetOps` trait 添加级联所需方法：

```rust
#[async_trait::async_trait]
trait ErasedSetOps: Send + Sync {
    // 现有方法不变...

    /// 从所有 Added 条目的 HasMany 导航抽取子实体。
    fn drain_cascade_children(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        meta: &EntityTypeMeta,
    ) -> Vec<crate::cascade::DrainedChild>;

    /// 添加级联抽取的子实体（类型擦除）到 DbSet，返回新条目索引。
    fn add_cascade_child(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        child: Box<dyn Any + Send + Sync>,
    ) -> Option<usize>;

    /// 返回追踪条目数。
    fn entry_count(&self, raw_set: &(dyn Any + Send + Sync)) -> usize;

    /// 读取 idx 处条目的第一个 PK 值（i64）。
    fn get_pk_at(&self, raw_set: &(dyn Any + Send + Sync), idx: usize) -> Option<i64>;

    /// 设置 idx 处条目指向 target_type 的 FK。
    fn set_fk_at(
        &self,
        raw_set: &mut (dyn Any + Send + Sync),
        idx: usize,
        target_type: TypeId,
        key: i64,
    );

    /// 阶段化保存（用于级联有序保存）。
    async fn insert_added(
        &self, conn: &mut (dyn IAsyncConnection + Send),
        provider: &dyn IDatabaseProvider,
        raw_set: &mut (dyn Any + Send + Sync),
        meta: &EntityTypeMeta,
    ) -> EFResult<usize>;

    async fn upsert_added(&self, ...) -> EFResult<usize>;
    async fn update_modified(&self, ...) -> EFResult<usize>;
    async fn delete_deleted(&self, ...) -> EFResult<usize>;
}
```

`SetOps<E>` 实现这些方法，downcast `Box<dyn Any>` → `DbSet<E>` / `E`：
- `drain_cascade_children`: 遍历 Added 条目，对每个 HasMany 导航调用 `entry.entity.drain_has_many(field)`
- `add_cascade_child`: `downcast::<E>(child)` → `db_set.add(child)`
- `get_pk_at`: `db_set.entries[idx].entity.key_values()` 取第一个 PK 值
- `set_fk_at`: `db_set.entries[idx].entity.set_foreign_key(target_type, key)`

更新 `SetOps<E>` 的 where 约束和 `DbContext::set<T>()` 的 where 约束，确保 `E: IGetKeyValues + INavigationSetter`（已有）。

### Step 7: 重写 `save_changes` 编排

**文件**: `crates/core/src/db_context.rs`

重写 `save_changes`（[db_context.rs:690-805](file:///d:/GitCode/RF/rust-ef/crates/core/src/db_context.rs#L690-L805)）：

```rust
pub async fn save_changes(&mut self) -> EFResult<SaveChangesResult> {
    let _save_guard = crate::observability::SaveChangesGuard::new();

    // 1. detect_changes（现有逻辑）
    let type_ids: Vec<TypeId> = self.sets.keys().copied().collect();
    for type_id in &type_ids { /* detect_changes */ }

    // 2. 构建 configured_metas（现有逻辑）
    let configured_metas: HashMap<TypeId, EntityTypeMeta> = /* ... */;

    // 3. 构建依赖图 + 拓扑排序
    let graph = DependencyGraph::build(&configured_metas);
    let insert_order = graph.topological_sort();
    let delete_order = graph.deletion_order();

    // 4. 迭代抽取级联子实体（处理自引用的任意深度）
    let mut fixup_links: Vec<FixupLink> = Vec::new();
    loop {
        let mut all_drained: Vec<DrainedChild> = Vec::new();
        for type_id in &type_ids {
            let saver = self.savers.get(type_id).unwrap();
            let set = self.sets.get_mut(type_id).unwrap();
            let meta = &configured_metas[type_id];
            all_drained.extend(saver.drain_cascade_children(set.as_mut(), meta));
        }
        if all_drained.is_empty() { break; }

        // 添加到子 DbSet + 记录 FixupLink
        for child in all_drained {
            let child_saver = self.savers.get(&child.child_type_id);
            let child_set = self.sets.get_mut(&child.child_type_id);
            // 如果子 DbSet 不存在，报错（用户必须预注册 ctx.set::<Child>()）
            let (child_saver, child_set) = match (child_saver, child_set) {
                (Some(s), Some(ss)) => (s, ss),
                _ => return Err(EFError::configuration(format!(
                    "Cascade target type {:?} not registered. Call ctx.set::<T>() before save.",
                    child.child_type_id
                ))),
            };
            let child_idx = child_saver.entry_count(child_set.as_ref());
            if child_saver.add_cascade_child(child_set.as_mut(), child.child).is_some() {
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
        // 下一轮迭代从新加入的子实体继续抽取（自引用深度处理）
    }

    // 5. 拦截器：on_saving
    let save_ctx = self.build_save_context();
    self.interceptor_pipeline.on_saving(&save_ctx).await?;

    // 6. 事务连接获取（现有逻辑）
    // ... TxnSource ...

    // 7. 按拓扑序 INSERT Added（非 upsert）
    let mut total_added = 0;
    let mut total_updated = 0;
    let mut total_deleted = 0;
    for type_id in &insert_order {
        if !self.sets.contains_key(type_id) { continue; }
        let saver = self.savers.get(type_id).unwrap();
        let set = self.sets.get_mut(type_id).unwrap();
        let meta = &configured_metas[type_id];
        total_added += saver.insert_added(conn, &*self.provider, set.as_mut(), meta).await?;

        // FK fixup：此类型的 Added 已插入，PK 已回填
        let mut pending_m2m: Vec<&FixupLink> = Vec::new();
        for link in &fixup_links {
            if link.parent_type_id == *type_id && link.through_table.is_none() {
                // 一对多 fixup：设置子 FK = 父 PK
                let parent_pk = saver.get_pk_at(set.as_ref(), link.parent_entry_idx);
                if let Some(pk) = parent_pk {
                    let child_saver = self.savers.get(&link.child_type_id).unwrap();
                    let child_set = self.sets.get_mut(&link.child_type_id).unwrap();
                    for &child_idx in &link.child_entry_indices {
                        child_saver.set_fk_at(
                            child_set.as_mut(), child_idx,
                            link.fk_target_type_id, pk,
                        );
                    }
                }
            } else if link.parent_type_id == *type_id && link.through_table.is_some() {
                pending_m2m.push(link);
            }
        }
        // M2M fixup 需要父子 PK 都回填——延迟到所有 INSERT 完成后
        // 暂存 pending_m2m 供后续处理
    }

    // 8. M2M 联表插入（所有 INSERT 完成后，PK 均已回填）
    for link in &fixup_links {
        if let Some(table) = &link.through_table {
            let parent_saver = self.savers.get(&link.parent_type_id).unwrap();
            let parent_set = self.sets.get(&link.parent_type_id).unwrap();
            let child_saver = self.savers.get(&link.child_type_id).unwrap();
            let child_set = self.sets.get(&link.child_type_id).unwrap();
            let parent_pk = parent_saver.get_pk_at(parent_set.as_ref(), link.parent_entry_idx);
            if let Some(pk) = parent_pk {
                let mut params: Vec<DbValue> = Vec::new();
                let mut count = 0;
                for &child_idx in &link.child_entry_indices {
                    if let Some(child_pk) = child_saver.get_pk_at(child_set.as_ref(), child_idx) {
                        params.push(DbValue::from(pk));
                        params.push(DbValue::from(child_pk));
                        count += 1;
                    }
                }
                if count > 0 {
                    let sql = crate::cascade::m2m_insert_sql(
                        table,
                        link.through_parent_fk_col.as_deref().unwrap_or(""),
                        link.through_child_fk_col.as_deref().unwrap_or(""),
                        count,
                    );
                    conn.execute(&sql, &params).await?;
                }
            }
        }
    }

    // 9. UPSERT Added（拓扑序）
    for type_id in &insert_order {
        // upsert_added ...
    }

    // 10. UPDATE Modified（拓扑序）
    for type_id in &insert_order {
        // update_modified ...
    }

    // 11. DELETE Deleted（逆拓扑序）
    for type_id in &delete_order {
        // delete_deleted ...
    }

    // 12. 事务提交 + accept_all_changes + 拦截器（现有逻辑）
    // ...
}
```

### Step 8: 测试

**文件**: `crates/core/tests/cascade_save_tests.rs`（新建）

测试用例：
1. `cascade_insert_blog_with_posts` — 一对多：Blog 带 2 个 Post，save 后 Post.blog_id 回填正确，Post PK 回填
2. `cascade_insert_self_referential_tree` — 自引用：Category 带 2 级子分类，save 后 parent_id 链正确
3. `cascade_insert_many_to_many` — M2M：Tag 带 2 个 Post，save 后 post_tags 联表行存在
4. `cascade_update_ordering` — Modified 父子按拓扑序更新
5. `cascade_delete_reverse_order` — Deleted 父子按逆拓扑序删除
6. `cascade_empty_has_many_noop` — 无导航的实体类型，级联抽取为空，无开销

测试实体定义复用 `examples/blog/src/entities.rs` 的 Blog/Post 模式。自引用测试需定义 Category 实体。M2M 测试需定义 Tag/Post/PostTag 实体。

---

## 5. 假设与决策

| 决策 | 选择 | 理由 |
|------|------|------|
| 抽取策略 | 自动抽取（EFCore 风格） | 用户选择，零额外用户代码 |
| 抽取范围 | 仅 Added 主体 | Unchanged/Modified 主体子实体已追踪 |
| 关系范围 | 一对多 + 自引用 + M2M | 用户选择"全部场景" |
| 抽取方向 | 仅 HasMany（父→子） | BelongsTo 反向抽取过于破坏性 |
| FK 设置机制 | `IGetKeyValues::set_foreign_key` | 复用已有 trait，宏已知道 FK 声明 |
| M2M 联表 | 直接 SQL（非实体追踪） | 容器不存储 Join 实体，直接 SQL 最简 |
| 删除级联 | 逆拓扑序删除（无 DeleteBehavior） | 用户要求"插入、更新"，DeleteBehavior 推迟 |
| 深度处理 | 迭代循环（非递归） | 自引用安全，无栈溢出 |
| 子 DbSet 注册 | 用户必须预调用 `ctx.set::<T>()` | Rust 编译时约束，无法运行时泛型创建 |

---

## 6. 验证步骤

1. `cargo check -p rust_ef_core` — 编译通过
2. `cargo check -p rust_ef_macros` — 宏编译通过
3. `cargo test -p rust_ef_core --test cascade_save_tests` — 6 个测试通过
4. `cargo test -p rust_ef_core` — 全量回归通过（除 PG 服务器依赖测试）
5. `cargo test -p rust_ef_sqlite` — SQLite 集成测试通过
6. 手动验证：Blog+Post 级联插入后，`ctx.set::<Post>().tracked_entries()` 返回 2 个 Post，blog_id 正确
7. 手动验证：M2M 级联插入后，联表行存在于数据库

---

## 7. 文件变更清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `crates/core/src/entity.rs` | 修改 | `IGetKeyValues::set_foreign_key`、`INavigationSetter::drain_has_many` |
| `crates/macros/src/entity.rs` | 修改 | 生成 `set_foreign_key` 和 `drain_has_many` impl |
| `crates/core/src/dependency_graph.rs` | 新建 | 依赖图 + Kahn 拓扑排序 |
| `crates/core/src/cascade.rs` | 新建 | DrainedChild、FixupLink、M2M SQL 生成 |
| `crates/core/src/db_context.rs` | 修改 | 扩展 ErasedSetOps、拆分 save_one_set 阶段、重写 save_changes |
| `crates/core/src/lib.rs` | 修改 | 注册新模块 |
| `crates/core/tests/cascade_save_tests.rs` | 新建 | 6 个级联测试 |
