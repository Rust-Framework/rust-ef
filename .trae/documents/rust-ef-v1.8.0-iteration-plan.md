# rust-ef 深度性能优化迭代计划 (v1.8.0)

> 基于 v1.7.0 热路径优化结果，针对 v1.7.0 明确推迟的 3 项分配瓶颈进行深度优化。
> 同时重构 ChangeTracker 为真实跟踪机制（消除残余 String 代码，单一跟踪源）。
> 目标版本：**v1.8.0**（含破坏性变更，提供迁移指南）。

---

## 当前状态分析

v1.7.0 完成了导航加载热路径（DbValueKey）、SQL 构建缓冲区、保存管线 scalar_props 提升 + 过滤器缓存。
本轮聚焦 **v1.7.0 推迟的 3 项** + **ChangeTracker 架构修复**：

| # | 瓶颈 | 位置 | 影响 | 根因 |
|---|------|------|------|------|
| 1 | `snapshot()` / `key_values()` HashMap 分配 | `entity.rs:83,59` + `gen.rs:208,186` | 高 | 每次调用分配 1 HashMap + N String 键 + N DbValue clone；22 个调用点 |
| 2 | `sql_cache` 键分配 | `executor_dml.rs:211` | 低-中 | per-row 路径仅在并发令牌存在时进入，where_clause 每实体不同 → 命中率 ≈ 0%，反而增加键分配开销 |
| 3 | ChangeTracker String 快照（残余代码） | `tracking.rs:54,63-66,91-180` | 中 | `detect_changes_with_properties` / `track_entity_with_snapshot` / `update_snapshot` 无生产代码调用；真正的跟踪在 DbSet 内联（`TrackedEntry.original: Option<HashMap<String, DbValue>>`） |

### 调用点分析（snapshot / key_values）

**22 个调用点**，主要模式：
- **HashMap 等值比较**：`db_set.rs:273` — `current == *original`
- **键查找**：`executor_dml.rs:279` — `snap.get(field_name)`；`navigation_loader.rs:172,196` — `e.snapshot().get(fk_column)`
- **迭代过滤**：`db_set.rs:279-283` — `current.iter().filter(|(k,v)| original.get(k) != Some(v))`
- **批量预计算**：`executor_dml.rs:88` — `entities.iter().map(|(e,_,_,_)| (e.snapshot(), e.key_values()))`
- **导航加载 PK 查找**：`navigation_loader.rs:137,161,241,305` — `e.key_values().get(ref_column)`

### 当前跟踪架构（问题）

```
DbContext
├── sets: HashMap<TypeId, Box<dyn Any>>          // DbSet<T> 实例
├── savers: HashMap<TypeId, Box<dyn ErasedSetOps>> // 类型擦除的操作接口
└── change_tracker: ChangeTracker                 // ⚠️ 残余 — save/detect 不使用

DbSet<T>
└── entries: Vec<TrackedEntry<T>>
    └── TrackedEntry { entity, state, original: Option<HashMap>, modified_properties, is_upsert }
        ↑ 真正的跟踪在这里（内联）

ChangeTracker（tracking.rs）
└── entries: Vec<TrackerEntry>
    └── TrackerEntry { snapshot: HashMap<String, PropertySnapshot{serialized: String}> }
        ↑ String 序列化，detect_changes_with_properties 无调用者 → 死代码
```

---

## 设计决策

### D1: `EntitySnapshot` 类型设计

**方案**：新建 `EntitySnapshot` 类型，内部用 `Box<[(&'static str, DbValue)]>`。

```rust
/// 不可变实体快照 — 单次堆分配，字段名为编译期 &'static str。
pub struct EntitySnapshot {
    entries: Box<[(&'static str, DbValue)]>,
}

impl EntitySnapshot {
    /// 按字段名查找值（O(n) 线性查找，典型实体 5-10 字段，性能影响可忽略）。
    pub fn get(&self, field: &str) -> Option<&DbValue> {
        self.entries.iter().find(|(k, _)| *k == field).map(|(_, v)| v)
    }

    /// 迭代所有 (field_name, value) 对。
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &DbValue)> {
        self.entries.iter().map(|(k, v)| (*k, v))
    }

    /// 字段数。
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

impl PartialEq for EntitySnapshot {
    fn eq(&self, other: &Self) -> bool {
        // 顺序无关比较：长度相同 + 每个键值对都能在对方找到匹配
        self.entries.len() == other.entries.len()
            && self.entries.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}
```

**收益**：
- **消除 HashMap 分配**：从 HashMap（至少 48 字节 + 桶数组）→ Box<[T]>（1 次堆分配，紧凑存储）
- **消除 String 键分配**：字段名是宏中 `stringify!(#ident)` 产生的 `&'static str`，零分配
- **不可变**：构造后不可修改，安全共享

**宏生成代码变更**：
```rust
// Before:
fn snapshot(&self) -> HashMap<String, DbValue> {
    let mut map = HashMap::new();
    map.insert("field1".to_string(), DbValue::from(self.field1.clone()));
    map.insert("field2".to_string(), DbValue::from(self.field2.clone()));
    map
}

// After:
fn snapshot(&self) -> EntitySnapshot {
    EntitySnapshot::from([
        ("field1", DbValue::from(self.field1.clone())),
        ("field2", DbValue::from(self.field2.clone())),
    ])
}
```

**否决方案**：
- ❌ `HashMap<&'static str, DbValue>`：仍保留 HashMap 分配，且 trait 对象生命周期复杂
- ❌ `Vec<(&'static str, DbValue)>`：可变性带来误用风险，Box<[T]> 更语义化（不可变快照）

### D2: ChangeTracker 重设计 — 单一跟踪源

**目标**：ChangeTracker 成为 entity state + original snapshot + modified_properties 的**唯一权威来源**，DbSet 仅持有 typed entity + entry_id 链接。

**架构变更**：

```
// Before: TrackedEntry 持有全部跟踪状态
TrackedEntry<T> { entity, state, original, modified_properties, is_upsert }

// After: DbSet 仅持有 entity + entry_id
DbSetEntry<T> { entity, entry_id }

// ChangeTracker 持有全部跟踪状态
TrackerEntry {
    id: u64,
    type_id: TypeId,
    state: EntityState,
    original: Option<EntitySnapshot>,   // 替代 String-based PropertySnapshot
    modified_properties: Vec<String>,
    is_upsert: bool,
}
```

**数据流**：

```
ctx.add::<T>(entity)
  → DbSet.push(DbSetEntry { entity, entry_id })
  → ChangeTracker.track(entry_id, Added, None)

ctx.attach::<T>(entity)
  → DbSet.push(DbSetEntry { entity, entry_id })
  → ChangeTracker.track(entry_id, Unchanged, Some(entity.snapshot()))

ctx.detect_changes()
  → for each DbSet:
      snapshots = db_set.entries.iter().map(|e| (e.entry_id, e.entity.snapshot()))
      change_tracker.detect_changes(snapshots)  // 比较并更新 state + modified_properties

ctx.save_changes()
  → for each DbSet:
      entries = db_set.entries.iter()
        .map(|e| (e.entity, change_tracker.entry(e.entry_id)))  // join by entry_id
      → execute_inserts/updates/deletes(entries)
```

**API 变更**（破坏性）：

| 变更 | Before | After | 迁移方式 |
|------|--------|-------|----------|
| DbSet 变更方法签名 | `db_set.add(entity)` | `ctx.add::<T>(entity)` | 用 DbContext 方法 |
| DbSet::attach | `db_set.attach(entity)` | `ctx.attach::<T>(entity)` | 用 DbContext 方法 |
| DbSet::detect_changes | `db_set.detect_changes()` | `ctx.detect_changes()` | 已有，无需改 |
| TrackedEntry 字段 | `.state` `.original` `.modified_properties` | 通过 ChangeTracker 查询 | 内部变更，用户不直接访问 |
| snapshot() 返回类型 | `HashMap<String, DbValue>` | `EntitySnapshot` | 宏自动处理；手动 impl 需更新 |
| key_values() 返回类型 | `HashMap<String, DbValue>` | `EntitySnapshot` | 宏自动处理；手动 impl 需更新 |

**DbContext 新增方法**：
```rust
impl DbContext {
    pub fn add<T: IEntityType>(&mut self, entity: T);
    pub fn attach<T: IEntityType>(&mut self, entity: T);
    pub fn update<T: IEntityType>(&mut self, entity: &mut T);  // 标记 Modified
    pub fn remove<T: IEntityType>(&mut self, index: usize) -> EFResult<()>;
}
```

**ErasedSetOps 签名变更**：
```rust
// Before:
fn detect_changes(&self, raw_set: &mut dyn Any);
fn collect_entries(&self, raw_set: &dyn Any) -> Vec<EntityEntryView>;

// After:
fn detect_changes(&self, raw_set: &mut dyn Any, tracker: &mut ChangeTracker);
fn collect_entries(&self, raw_set: &dyn Any, tracker: &ChangeTracker) -> Vec<EntityEntryView>;
fn collect_for_save(
    &self, raw_set: &dyn Any, tracker: &ChangeTracker, state: EntityState
) -> Vec<ErasedSaveEntry>;  // 类型擦除的 (entity_ref, original, modified_props)
```

### D3: `sql_cache` 移除

**方案**：直接移除 `execute_updates_per_row` 中的 `sql_cache`。

**理由**：per-row 路径仅在并发令牌存在或复合 PK 时进入。当并发令牌存在时，`where_clause` 包含令牌值 → 每实体不同 → 缓存命中率 ≈ 0%。键分配（2 String + 1 Vec）反而增加开销。

**变更**：
```rust
// Before:
let sql = sql_cache
    .entry((table_name.to_string(), set_cols.iter().map(|s| (*s).to_string()).collect(), where_clause.clone()))
    .or_insert_with(|| gen.update(table_name, &set_cols, &where_clause))
    .clone();

// After:
let sql = gen.update(table_name, &set_cols, &where_clause);
```

---

## 迭代计划

### 迭代 1：EntitySnapshot 类型 + trait 变更（P1）

**目标**：引入 `EntitySnapshot` 类型，消除 `snapshot()` / `key_values()` 的 HashMap + String 键分配。

**变更清单**：

1. **新建 `crates/core/src/entity_snapshot.rs`**
   - `EntitySnapshot` 结构体（`Box<[(&'static str, DbValue)]>`）
   - `get()` / `iter()` / `len()` / `is_empty()`
   - `PartialEq` 实现（顺序无关）
   - `From<Vec<(&'static str, DbValue)>>` / `From<&[(&'static str, DbValue)]>` 构造
   - `Debug` 实现
   - 单元测试：构造、get、iter、eq、空快照

2. **修改 `crates/core/src/entity.rs`**
   - `IEntitySnapshot::snapshot()` 返回 `EntitySnapshot`
   - `IGetKeyValues::key_values()` 返回 `EntitySnapshot`

3. **修改 `crates/core/src/lib.rs` / `mod.rs`**
   - 导出 `EntitySnapshot`

4. **修改 `crates/macros/src/entity/gen.rs`**
   - `snapshot()` 生成代码：`EntitySnapshot::from([...])` 替代 `HashMap::new() + insert`
   - `key_values()` 同理

5. **更新 22 个调用点**（`crates/core/src/`）：
   - `db_set.rs:272,353,433` — `entity.snapshot()` 返回值变更，`==` 比较用 `EntitySnapshot::eq`
   - `db_set.rs:279-283` — `current.iter().filter(...)` 改用 `EntitySnapshot::iter()`
   - `executor_dml.rs:88,226,227` — `snap.get(field)` 改用 `EntitySnapshot::get()`
   - `executor_dml.rs:341,433` — `keys.get(pk_field)` 同理
   - `executor.rs:73,209` — `snap.get(field)` 同理
   - `navigation_loader.rs:137,161,172,196,241,305` — `.get()` 同理
   - `model_builder.rs:401` — `e.snapshot()` 收集
   - `set_ops.rs:225,244,344,439,447` — `.key_values().get()` / `.snapshot()` 同理
   - `build_where_with_concurrency` (executor_dml.rs:462) — 参数类型 `&HashMap` → `&EntitySnapshot`

6. **修改 `TrackedEntry.original`**：`Option<HashMap<String, DbValue>>` → `Option<EntitySnapshot>`

**验证**：
- `cargo test --workspace --all-features` 全量通过
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 无警告
- `cargo fmt --all -- --check` 通过
- 新增 EntitySnapshot 单元测试通过
- 现有 bench 编译通过（API 变更后）

---

### 迭代 2：ChangeTracker 重设计 — 单一跟踪源（P1）

**目标**：ChangeTracker 成为跟踪状态的唯一权威来源，DbSet 仅持有 typed entity。消除残余 String 代码。

**变更清单**：

1. **修改 `crates/core/src/tracking.rs`**
   - 删除 `PropertySnapshot` 结构体（String-based）
   - 删除 `track_entity_with_snapshot` / `detect_changes_with_properties` / `update_snapshot`（死代码）
   - `TrackerEntry` 改为：`{ id, type_id, state, original: Option<EntitySnapshot>, modified_properties, is_upsert }`
   - 新增 `track(&mut self, entry_id, type_id, state, original: Option<EntitySnapshot>)`
   - 新增 `detect_changes(&mut self, current_snapshots: &[(u64, EntitySnapshot)])`
     — 比较 `current` vs `original`，更新 `state` + `modified_properties`
   - 新增 `entry_state(&self, entry_id) -> EntityState`
   - 新增 `entry_original(&self, entry_id) -> Option<&EntitySnapshot>`
   - 新增 `entry_modified(&self, entry_id) -> &[String]`
   - 更新 `accept_all_changes` / `reject_all_changes` / `has_changes` / `entries` / `count_by_state`

2. **修改 `crates/core/src/db_set.rs`**
   - `TrackedEntry<T>` → `DbSetEntry<T> { entity: T, entry_id: u64 }`
   - 移除 `state` / `original` / `modified_properties` / `is_upsert` 字段
   - `add()` / `attach()` / `upsert()` 需要 ChangeTracker 引用 → 改为 DbContext 方法（见下）
   - `detect_changes()` 需要 ChangeTracker 引用 → 委托给 ChangeTracker
   - `tracked_by_state()` 需要 ChangeTracker 引用 → join by entry_id
   - `accept_all_changes()` 需要 ChangeTracker 引用

3. **修改 `crates/core/src/db_context/context.rs`**
   - 新增 `ctx.add::<T>(entity)` / `ctx.attach::<T>(entity)` / `ctx.update::<T>(&mut entity)` / `ctx.remove::<T>(index)`
   - `detect_changes()` 编排：收集 DbSet 快照 → 传给 ChangeTracker → 更新状态
   - `save_changes()` 编排：join DbSet (entity) + ChangeTracker (state, original, modified_props)

4. **修改 `crates/core/src/db_context/set_ops.rs`**
   - `ErasedSetOps` 方法签名增加 `&mut ChangeTracker` / `&ChangeTracker` 参数
   - `detect_changes(&self, raw_set, tracker)`
   - `collect_entries(&self, raw_set, tracker) -> Vec<EntityEntryView>`
   - 新增 `collect_for_save(&self, raw_set, tracker, state) -> Vec<ErasedSaveEntry>`
   - `accept_all_changes(&self, raw_set, tracker)`

5. **修改 `crates/core/src/db_context/save_pipeline.rs` + `save_phases.rs`**
   - 从 `(entity, original, modified_properties)` 元组改为 join by entry_id
   - 使用 `ErasedSetOps::collect_for_save` 获取 save 数据

6. **修改 `crates/core/src/change_executor/executor_dml.rs`**
   - `execute_updates` / `execute_deletes` 参数中 `Option<&HashMap<String, DbValue>>` → `Option<&EntitySnapshot>`

7. **更新测试文件**
   - 所有 `db_set.add(entity)` → `ctx.add::<T>(entity)`
   - 所有 `db_set.attach(entity)` → `ctx.attach::<T>(entity)`

8. **更新 examples/**
   - `examples/blog/` / `examples/soft_delete/` / `examples/audit/` 的 API 调用

**验证**：
- `cargo test --workspace --all-features` 全量通过
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 无警告
- `cargo fmt --all -- --check` 通过
- 所有 `.rs` 文件 ≤500 行
- `tracking.rs` 无 String-based 残余代码

---

### 迭代 3：sql_cache 移除 + 连带优化（P2）

**目标**：移除 per-row UPDATE 中命中率 ≈ 0% 的 `sql_cache`。

**变更清单**：

1. **修改 `crates/core/src/change_executor/executor_dml.rs`**
   - 移除 `sql_cache: HashMap<(String, Vec<String>, String), String>`
   - 直接调用 `gen.update(table_name, &set_cols, &where_clause)`
   - 移除 `HashMap` import（如不再使用）

2. **检查 per-row DELETE 路径**（`execute_deletes_per_row`）
   - 确认无 sql_cache（已确认 — 直接调用 `gen.delete()`）
   - 检查是否有其他可优化点

3. **检查迭代 1-2 引入的代码是否有额外分配热点**
   - `EntitySnapshot::eq` 的 O(n²) 比较 — 确认对大实体（20+ 字段）是否需要优化
   - `detect_changes` 中的 `current_snapshots: Vec<(u64, EntitySnapshot)>` 收集 — 可否用迭代器替代

**验证**：
- `cargo test --workspace --all-features` 全量通过
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` 无警告
- `cargo bench -p rust-ef --bench bench_save` 对比 v1.7.0 基线

---

### 迭代 4：基准测试 + v1.8.0 发布（P3）

**目标**：量化性能提升，更新文档，发布 v1.8.0。

**变更清单**：

1. **新增/更新基准测试**
   - `bench_snapshot.rs` — 直接基准 `entity.snapshot()` 和 `entity.key_values()` 的分配开销
   - 更新 `bench_detect_changes.rs` — 适配新的 ChangeTracker API
   - 更新 `bench_save.rs` — 适配新的 DbContext API

2. **新增 `docs/v1.8-migration-guide.md`**
   - DbContext API 变更：`db_set.add()` → `ctx.add::<T>()`
   - `snapshot()` / `key_values()` 返回类型变更
   - 手动 `IEntitySnapshot` impl 迁移指南

3. **更新 `CHANGELOG.md`**
   - `[1.8.0]` 条目

4. **版本号 bump**
   - workspace `1.7.0` → `1.8.0`
   - 所有依赖约束更新

5. **验证流程**
   - `cargo check --workspace --all-features`
   - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
   - `cargo fmt --all -- --check`
   - `cargo test --workspace --all-features`
   - `cargo bench -p rust-ef`（记录所有基准结果）
   - `cargo publish --dry-run --allow-dirty`

6. **发布流程**（用户确认后执行）
   - `git commit` + `git tag v1.8.0`
   - `cargo publish`（按依赖序）
   - `git push`

---

## 假设与约束

1. **EntitySnapshot O(n) 查找可接受**：典型实体 5-10 字段，`get()` 线性查找开销远小于 HashMap 分配 + String 键分配。对 20+ 字段的大实体，`PartialEq` 的 O(n²) 比较可能需关注（可在迭代 3 评估）。
2. **DbContext API 变更可接受**：`db_set.add()` → `ctx.add::<T>()` 是破坏性变更，但更符合 EFCore 模式（DbContext 是操作入口），且 v1.8.0 是 minor version，可含迁移指南。
3. **entry_id join 开销可忽略**：ChangeTracker 的 `entries: HashMap<u64, TrackerEntry>` 提供 O(1) 查找，DbSet → ChangeTracker 的 join 是 O(n)。
4. **所有文件 ≤500 行**：`tracking.rs` 当前 288 行，重设计后预计 ~250 行（删多于增）。`db_set.rs` 当前 ~440 行，移除跟踪字段后预计 ~350 行。
5. **mod.rs 规范**：`entity_snapshot.rs` 加入 `entity/mod.rs` 或 `lib.rs` 的 `mod` + `pub use`。

---

## 风险与回退

| 风险 | 缓解 |
|------|------|
| EntitySnapshot::eq O(n²) 对大实体慢 | 迭代 3 评估；必要时用 hash 预计算 |
| ChangeTracker 重设计影响面大（ErasedSetOps、save_pipeline、测试、examples） | 迭代 2 拆分为子步骤，每步编译验证 |
| DbContext API 变更破坏用户代码 | 提供迁移指南；保留 `db_set.add()` 作为 deprecated wrapper（1 minor 过渡期） |
| entry_id join 引入新瓶颈 | ChangeTracker 用 HashMap<u64, TrackerEntry>，O(1) 查找 |
| 宏生成代码变更影响 feature-gated 变体 | 编译测试覆盖 chrono/uuid/decimal feature 组合 |

---

## 验收标准

- [ ] 迭代 1：`snapshot()` / `key_values()` 返回 `EntitySnapshot`；无 HashMap 分配；22 调用点全部适配
- [ ] 迭代 2：ChangeTracker 为唯一跟踪源；DbSet 仅持有 entity + entry_id；无 String-based 残余代码
- [ ] 迭代 3：`sql_cache` 已移除；per-row 路径无缓存键分配
- [ ] 迭代 4：所有基准测试通过；v1.8.0 发布到 crates.io
- [ ] 全程：`cargo check` / `clippy` / `fmt` / `test` 四项全绿
- [ ] 所有 `.rs` 文件 ≤500 行
- [ ] 迁移指南文档完整
