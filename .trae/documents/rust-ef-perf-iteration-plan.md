# rust-ef 性能优化迭代计划 (v1.7.0)

> 基于 v1.6.0 清理结果，针对代码探索中识别的热路径分配瓶颈，规划本轮性能优化。
> 目标版本：**v1.7.0**（破坏性变更尽量规避；如有则显式标注）。

---

## 当前状态分析

v1.6.0 已完成代码清理（文件拆分 ≤500 行、panic 收敛、RwLock 化、EFErrorCode、dead_code 标注）。
本轮聚焦 **热路径上的堆分配**，通过代码审查已定位以下瓶颈（按影响降序）：

| # | 瓶颈 | 位置 | 影响 | 根因 |
|---|------|------|------|------|
| 1 | `db_value_key()` | `navigation_loader.rs:324` | 高 | `format!("{}", v)` 每键分配 String，50×10 include 场景 = 500 次分配 |
| 2 | `hex::encode` | `db_value.rs:70-74` | 高 | 每字节 `format!("{:02x}", b)` 分配 String 再 collect |
| 3 | `to_sql_with()` | `state.rs:108-308` | 中 | 大量 `format!()` 临时 String，再 `push_str` 拷贝 |
| 4 | `scalar_props` 重收集 | `executor_dml.rs:201,393` | 中 | per-row 路径每实体 `mapped_scalar_properties().collect()` |
| 5 | 查询过滤器重编译 | `executor_dml.rs:149,355` 等 | 低-中 | `compile_bool_expr` 每 batch 调用；SQLite/MySQL 的 `?` 占位符其实与索引无关 |
| 6 | `sql_cache` 键分配 | `executor_dml.rs:247-254` | 低 | `(String, Vec<String>, String)` 每实体分配 2 String + 1 Vec |

**不在本轮范围**：
- `snapshot()` 返回 `HashMap<String, DbValue>` —— 属设计固有，需 trait 变更，影响面过大，推迟。
- `ChangeTracker` 的 String 快照 —— 非主热路径（`DbSet::detect_changes` 直接用 HashMap 比较）。
- 连接池获取 —— 已由连接池管理，非瓶颈。

---

## 设计决策

### D1: `DbValueKey` 方案选择

**问题**：`DbValue` 含 `f32`/`f64`，无法 `derive(Eq, Hash)`（NaN != NaN）。需一个可用作 HashMap 键的类型。

**方案**：新建 `DbValueKey` 枚举（owned），将 `&DbValue` 转换为可哈希形式。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DbValueKey {
    Null,
    Bool(bool),
    Int(i64),           // I16/I32/I64 统一为 i64（PK 最常见类型，零分配）
    FloatBits(u64),     // F32/F64 经 to_bits() 统一（哈希语义正确）
    Str(String),        // String 仍需 clone，但免去 format! 机制开销
    Bytes(Vec<u8>),     // Bytes 仍需 clone
    #[cfg(feature = "chrono")]
    DateTime(i64),      // timestamp -> i64
    #[cfg(feature = "chrono")]
    NaiveDateTime(i64),
    #[cfg(feature = "chrono")]
    NaiveDate(i32),
    #[cfg(feature = "uuid")]
    Uuid(u128),
    #[cfg(feature = "decimal")]
    Decimal(String),    // 序列化为字符串（rust_decimal 无 Hash）
}
```

**收益**：
- 整数 PK（最常见）：**零分配**（i64 是 Copy）
- 浮点：零分配（u64 是 Copy），且 `to_bits()` 保证 NaN 哈希一致
- String/Bytes：仍有一次 clone，但免去 `format!` 的格式串解析 + Display 调用 + 引号包裹

**否决方案**：
- ❌ 直接为 `DbValue` impl `Eq+Hash`：`Eq` 要求 `==` 自反，但 `f32::NAN != f32::NAN`，soundness 违例。
- ❌ `DbValueKey<'a>(&'a DbValue)` 借用式包装：HashMap 键需借用值内部数据，产生自引用（键引用行，行存在值中），生命周期无法表达。
- ❌ 外部 `hex` crate：现有本地 `hex` 模块仅需查表优化，不值得引入依赖。

### D2: `hex::encode` 优化方式

**方案**：查表法，预分配 `String::with_capacity(bytes.len() * 2)`，单次分配。

```rust
mod hex {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(HEX_CHARS[(b >> 4) as usize] as char);
            s.push(HEX_CHARS[(b & 0x0f) as usize] as char);
        }
        s
    }
}
```

**收益**：N 字节从 N+1 次分配降为 1 次分配，且无 `format!` 机制开销。

### D3: `to_sql_with()` 优化策略

**方案**：将 `format!("{} FROM {}", select, self.from)` 等模式改为 `String::with_capacity` 预分配 + `push_str` / `write!` 到缓冲区。

**注意**：不改变 SQL 语义，不重排子句顺序。仅消除中间 String 分配。

### D4: `scalar_props` 提升范围

**方案**：在 `execute_updates_per_row` 和 `execute_deletes_per_row` 中，将 `meta.mapped_scalar_properties().collect()` 提升到循环外（使用 `entities[0].1` 的 meta，与 batch 路径一致）。

**前提**：同一 `execute_updates_per_row` 调用中所有实体类型相同（`&[&E, ...]` 约束），meta 引用指向同一 `EntityTypeMeta`。

### D5: 查询过滤器 SQL 缓存

**方案**：为 `ISqlGenerator` 添加 `uses_numbered_placeholders()` 方法：
- PostgreSQL `$N` → `true`（每 batch 起始索引不同，必须重编译）
- SQLite/MySQL `?` → `false`（占位符与索引无关，可缓存）

当 `uses_numbered_placeholders() == false` 且 `query_filter` 不变时，缓存 `compile_bool_expr` 结果供后续 batch 复用。

---

## 迭代计划

### 迭代 1：导航加载热路径优化（P1）

**目标**：消除 `db_value_key()` 和 `hex::encode` 的堆分配。

**变更清单**：

1. **新建 `crates/core/src/provider/db_value_key.rs`**
   - 定义 `DbValueKey` 枚举（见 D1）
   - 实现 `From<&DbValue> for DbValueKey`
   - 实现 `From<&[DbValue]>` 批量转换辅助（可选）
   - 单元测试：整数/浮点/String/Bytes/Null 转换 + 哈希一致性

2. **修改 `crates/core/src/provider/mod.rs`**
   - `mod db_value_key;` + `pub use db_value_key::DbValueKey;`

3. **修改 `crates/core/src/provider/db_value.rs`**
   - 优化 `mod hex` 为查表法（见 D2）
   - 补充测试：空字节、单字节、多字节、与原实现输出一致

4. **修改 `crates/core/src/navigation_loader.rs`**
   - `db_value_key()` 函数返回类型 `String` → `DbValueKey`
   - `group_rows` / `group_join_rows` / `index_rows` 返回 `HashMap<DbValueKey, ...>`
   - `load_scalar_navigation` / `load_many_to_many` 中的查找键改用 `DbValueKey::from(pk)`
   - `load_many_to_many` 中 `HashSet<String>` 去重改用 `HashSet<DbValueKey>`
   - 验证：所有 `format!("{}", v)` 调用点已消除

5. **修改 `crates/core/src/lib.rs`**（如需导出）
   - `pub use crate::provider::DbValueKey;`（仅在 prelude 需要时）

**验证**：
- `cargo test -p rust-ef` 全部通过
- `cargo bench -p rust-ef --bench bench_include` 对比 v1.6.0 基线，include 路径性能提升
- `cargo clippy --all --all-features -- -D warnings` 无警告
- `cargo fmt --all -- --check` 通过

---

### 迭代 2：SQL 构建缓冲区优化（P2）

**目标**：消除 `to_sql_with()` 中的中间 String 分配。

**变更清单**：

1. **修改 `crates/core/src/query/state.rs` — `to_sql_with()`**
   - 开头 `let mut sql = String::with_capacity(256);`（保守初始容量，按需增长）
   - SELECT 子句：用 `write!(sql, "SELECT {}{}", ...)` 或 `push_str` 替代 `format!`
   - FROM：`sql.push_str(" FROM "); sql.push_str(&self.from);`
   - JOIN：循环中 `sql.push_str(" "); sql.push_str(&join.to_sql());`
   - WHERE：`sql.push_str(" WHERE "); sql.push_str(&compiled);`
   - GROUP BY / HAVING / ORDER BY：同理
   - LIMIT/OFFSET：`sql.push(' '); sql.push_str(&pagination);`
   - CTE 前缀：改为 `sql.insert_str(0, &prefix)` 或构建后 prepend
   - Set 操作：`sql.push_str(kw); sql.push_str(&op.operand_sql);`

2. **修改 `crates/core/src/provider/traits.rs` — `update_batch()` 默认实现**
   - 同样用 `String::with_capacity` + `push_str` 替代 `format!`
   - CASE WHEN 片段用 `write!` 写入缓冲区

3. **检查 `crates/core/src/query/ast.rs`、`having_expr.rs`、`window.rs` 的 `to_sql()` 方法**
   - 对高频调用的 `to_sql()` 应用同样优化（仅当确认是热路径时）

**验证**：
- `cargo test -p rust-ef` 全部通过（SQL 输出字符串完全一致）
- 新增测试：对比 `to_sql_with()` 输出与 v1.6.0 快照字符串（逐字节相等）
- `cargo clippy --all --all-features -- -D warnings` 无警告
- `cargo bench -p rust-ef --bench bench_query` 对比基线

---

### 迭代 3：保存管线优化（P2）

**目标**：消除 per-row 路径的 `scalar_props` 重收集 + 查询过滤器重编译。

**变更清单**：

1. **修改 `crates/core/src/change_executor/executor_dml.rs` — `execute_updates_per_row()`**
   - 将 `let scalar_props: Vec<_> = meta.mapped_scalar_properties().collect();` 移到 `for` 循环外
   - 使用 `entities[0].1` 获取 meta（与 batch 路径模式一致）
   - 循环内复用预收集的 `scalar_props`

2. **修改 `crates/core/src/change_executor/executor_dml.rs` — `execute_deletes_per_row()`**
   - 同样将 `scalar_props` 收集移到循环外

3. **修改 `crates/core/src/provider/traits.rs` — `ISqlGenerator`**
   - 添加方法：`fn uses_numbered_placeholders(&self) -> bool { false }`
   - PostgreSQL 实现覆写为 `true`；SQLite/MySQL 保持默认 `false`

4. **修改各 provider 的 SQL generator 实现**
   - `crates/postgres/src/sql_generator.rs`：impl `uses_numbered_placeholders() -> true`
   - SQLite/MySQL 无需改动（使用默认 `false`）

5. **修改 `crates/core/src/change_executor/executor_dml.rs` — batch UPDATE/DELETE**
   - 在 batch 循环外预编译过滤器 SQL（当 `!gen.uses_numbered_placeholders()` 时）：
     ```rust
     let cached_filter_sql: Option<String> = if !gen.uses_numbered_placeholders() {
         query_filter.map(|f| {
             let mut idx = usize::MAX; // 占位，实际用 ? 不依赖索引
             compile_bool_expr(f, gen, &mut idx)
         })
     } else { None };
     ```
   - batch 循环内：若 `cached_filter_sql` 存在则直接复用，否则按原逻辑编译
   - 同样应用于 `execute_deletes` 的 batch 路径

6. **修改 `crates/core/src/navigation_loader.rs` — `apply_filter_to_sql()`**
   - 当 `!gen.uses_numbered_placeholders()` 时，过滤器 SQL 编译结果可缓存
   - 由于 `apply_filter_to_sql` 每次调用是独立的 include 路径，缓存收益有限——仅当同一 `filter_map` 跨多次调用时有效
   - **决策**：本轮仅添加 `uses_numbered_placeholders()` 方法并应用于 executor_dml，navigation_loader 暂不缓存（其调用频率低）

**验证**：
- `cargo test -p rust-ef` 全部通过
- 新增测试：验证 per-row update/delete 在 1000 实体下 `scalar_props` 只收集 1 次（可通过计数或性能断言）
- `cargo clippy --all --all-features -- -D warnings` 无警告
- `cargo bench -p rust-ef --bench bench_insert` 对比基线

---

### 迭代 4：基准测试 + 发布（P3）

**目标**：量化性能提升，补充基准测试，发布 v1.7.0。

**变更清单**：

1. **新增 `crates/core/benches/bench_save.rs`**
   - 场景：1000 实体批量 INSERT / UPDATE / DELETE
   - 对比 v1.6.0 基线（通过 git stash 或分支对比）

2. **新增 `crates/core/benches/bench_detect_changes.rs`**
   - 场景：1000 已跟踪实体，500 有修改，运行 `detect_changes`
   - 量化 HashMap 比较开销（基线参考，本轮不优化但建立基线）

3. **更新 `CHANGELOG.md`**
   - 新增 `[1.7.0]` 条目，记录性能优化
   - 标注 `DbValueKey` 为新增公开类型

4. **更新 `Cargo.toml`**
   - `version = "1.6.0"` → `"1.7.0"`

5. **验证流程**
   - `cargo check --all --all-features`
   - `cargo clippy --all --all-features -- -D warnings`
   - `cargo fmt --all -- --check`
   - `cargo test --all --all-features`
   - `cargo bench -p rust-ef`（记录所有基准结果）
   - `cargo publish --dry-run --allow-dirty`（按序：macros → core → postgres/mysql/sqlite → cli）

6. **发布流程**（用户确认后执行）
   - `git commit -F` (CHANGELOG + 版本号 + 优化代码)
   - `git tag v1.7.0`
   - `cargo publish`（按依赖序）
   - `git push origin main; git push origin v1.7.0`

**验证**：
- 所有基准测试通过且无回归
- crates.io 发布成功
- `cargo doc --all-features` 无断链（`DbValueKey` 文档完整）

---

## 假设与约束

1. **整数 PK 假设**：大多数实体的 PK/FK 为 `i32`/`i64`，`DbValueKey::Int(i64)` 覆盖此场景且零分配。String PK 仍有一次 clone（但优于 `format!`）。
2. **per-row 路径 meta 一致性**：`execute_updates_per_row`/`execute_deletes_per_row` 中所有实体共享同一 `EntityTypeMeta`（由 `&E` 类型约束保证）。
3. **过滤器 SQL 缓存安全性**：当 `uses_numbered_placeholders() == false` 时，`compile_bool_expr` 输出仅含 `?`，与参数索引无关，可安全缓存。
4. **不破坏公开 API**：`DbValueKey` 为新增类型；`ISqlGenerator::uses_numbered_placeholders()` 有默认实现，不破坏现有 provider 实现。
5. **文件行数约束**：所有新增/修改文件保持 ≤500 行。`db_value_key.rs` 预计 ~120 行；`navigation_loader.rs` 改动后预计 ~370 行。
6. **mod.rs 规范**：新增 `db_value_key` 模块时，`provider/mod.rs` 仅添加 `mod` 声明和 `pub use`。

---

## 风险与回退

| 风险 | 缓解 |
|------|------|
| `DbValueKey` 转换遗漏变体（feature-gated） | 用 `#[cfg(feature = ...)]` 严格对齐 `DbValue` 变体；编译测试覆盖所有 feature 组合 |
| `to_sql_with` 重构导致 SQL 输出变化 | 新增字符串快照测试，逐字节对比 v1.6.0 输出 |
| 过滤器缓存导致参数顺序错乱 | 缓存仅用于 SQL 片段，参数仍按原逻辑收集；测试验证多 batch 场景 |
| 基准测试波动 | 使用 criterion 统计分析，sample_size ≥ 20，报告 p-value |

---

## 验收标准

- [x] 迭代 1：`db_value_key()` 不再调用 `format!`；`hex::encode` 单次分配
- [x] 迭代 2：`to_sql_with()` 无中间 `format!` 临时 String（clippy 无 large_enum_variant 警告）
- [x] 迭代 3：per-row 路径 `scalar_props` 每调用只收集 1 次；`uses_numbered_placeholders()` 已添加
- [x] 迭代 4：所有基准测试通过；v1.7.0 发布到 crates.io
- [x] 全程：`cargo check` / `clippy` / `fmt` / `test` 四项全绿
