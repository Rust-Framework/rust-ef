# 架构模块化推进计划:linq/ 收尾 + migration/ 子目录化

## 概述

延续架构模块化工作,分两阶段推进:

- **Phase A(收尾)**:完成上轮 linq/ 拆分的待办事项 —— 运行 `cargo test --workspace` 验证 + 更新 CHANGELOG.md。
- **Phase B(主任务)**:将 `crates/core/src/migration.rs`(1449 行,9 个职责混在一起)拆分为 `migration/` 子目录,共 9 个子模块,每个模块单一职责、≤350 行。

**目标**:职责清晰、模块化开发,避免单文件堆积大量逻辑,确保架构可演进、可维护、稳定。

---

## 当前状态分析

### Phase A 待办

上轮会话已完成 linq/ 拆分(6 个子模块,`linq.rs` 已删除,`cargo check --workspace` 通过),但:

| 待办 | 状态 | 验证方式 |
|------|------|---------|
| `cargo test --workspace` | ❌ 未运行 | 所有测试通过(含 linq! 查询/CTE 测试) |
| CHANGELOG.md 条目 | ❌ 未追加 | 记录 linq.rs 拆分 + E0027 修复 |

### Phase B 目标文件:`crates/core/src/migration.rs`(1449 行)

**精确结构映射**(基于 grep + Read 验证):

| 行号范围 | 内容 | 职责 | 目标模块 |
|---------|------|------|---------|
| L1-11 | 文件头注释 + imports | — | 分散到各子模块 |
| L13-57 | `Migration` / `ModelSnapshot` / `SnapshotEntityType` / `SnapshotColumn` | 数据类型 | **types.rs** |
| L59-160 | `MigrationDialect` enum + impl(`quote`、`map_column_type`) | 方言类型映射 | **types.rs** |
| L163-206 | `SchemaChange` enum(pub(crate)) | 内部 diff 结果类型 | **types.rs** |
| L208-212 | `MigrationEngine` struct | 引擎类型定义 | **engine.rs** |
| L214-442 | 第一 `impl MigrationEngine` 块:`new`/`generate`/`create_snapshot`/`initial_create`/`diff`/`append_create_table_fks`/`append_create_table_indexes` | diff 入口 + 快照生成 | **engine.rs** |
| L443-457 | `fk_target` / `index_name`(自由函数) | diff 辅助 | **diff.rs** |
| L458-549 | `fk_reference_for_property` / `diff_foreign_keys` | FK diff | **diff.rs** |
| L550-589 | `columns_structurally_equal` / `diff_indexes` | 列/索引 diff | **diff.rs** |
| L590-604 | `IndexKind` enum + `index_kind` fn | 索引分类 | **diff.rs** |
| L606-948 | 第二 `impl MigrationEngine` 块(SQL 部分):`initial_create_with_fks`/`foreign_key_name`/`index_name`/`generate_alter_column_sql`/`generate_up_sql`/`generate_ddl_sql`/`generate_up_sql_inner`/`generate_down_sql` | SQL 生成 | **engine_sql.rs** |
| L949-1253 | 第二 `impl MigrationEngine` 块(执行部分):`ensure_history_table`/`apply`/`revert`/`get_applied_migrations`/`is_applied`/`apply_pending`/`revert_last`/`revert_to_target`/`generate_script`/`ensure_created`/`ensure_deleted`/`apply_seed_data` | 异步执行 + 生命周期 | **engine_exec.rs** |
| L1260-1269 | `MigrationHistoryEntry` + `MIGRATION_HISTORY_TABLE` + `PRODUCT_VERSION` 常量 | 历史表元数据 | **types.rs** |
| L1271-1339 | `seed_insert_sql` / `split_sql_statements` / `create_migration_history_table_sql` | 历史 SQL 辅助 | **history.rs** |
| L1341-1423 | `MigrationStore` struct + impl | 文件系统 I/O | **store.rs** |
| L1425-1551 | `parse_model_snapshot_json` / `migration_io_err` / `snapshot_to_json` / `snapshot_from_json` / `extract_json_string` / `extract_quoted_after_colon` | JSON 序列化 | **snapshot.rs** |

### 公共 API 约束(必须保持向后兼容)

`crates/core/src/lib.rs:35` 声明 `pub mod migration;`,以下路径被外部引用(grep 验证):

| 公共路径 | 引用方 |
|---------|--------|
| `rust_ef::migration::MigrationDialect` | sqlite/postgres/mysql provider、9 个测试文件 |
| `rust_ef::migration::MigrationEngine` | cli/main.rs、db_context.rs、8 个测试文件 |
| `rust_ef::migration::MigrationEngine::new` | cli、tests |
| `rust_ef::migration::MigrationEngine::foreign_key_name` | integration_tests.rs |
| `rust_ef::migration::Migration` | cli、migration_cli_tests |
| `rust_ef::migration::SnapshotColumn` | integration_tests、extended_types_tests |
| `rust_ef::migration::MigrationStore` | production_tests |
| `rust_ef::migration::create_migration_history_table_sql` | sqlite_crud_tests |

`mod.rs` 必须通过 `pub use` 重新导出所有这些类型,保持 `rust_ef::migration::*` 路径不变。

### 模块依赖关系(自底向上,无循环)

```
types.rs        (无内部依赖,仅 crate::error、crate::metadata)
    ↑
diff.rs         (依赖 types::{SchemaChange, SnapshotColumn, IndexKind})
    ↑
engine.rs       (依赖 types::*, diff::*; 含 MigrationEngine struct + 第一 impl 块)
    ↑
engine_sql.rs   (impl MigrationEngine,依赖 types::*, engine::MigrationEngine)
    ↑
engine_exec.rs  (impl MigrationEngine,依赖 types::*、history::*、engine_sql 间接)
    ↑
history.rs      (依赖 types::MigrationDialect;使用 crate::provider::ISqlGenerator)
store.rs        (依赖 types::{Migration, ModelSnapshot}、snapshot::*、mod::migration_io_err)
snapshot.rs     (依赖 types::{ModelSnapshot, SnapshotEntityType, SnapshotColumn})
    ↑
mod.rs          (re-export 全部公共项 + pub(crate) fn migration_io_err)
```

**说明**:Rust 允许同一类型的 `impl` 块分布在多个文件中。`MigrationEngine` 的 struct 定义在 `engine.rs`,SQL 生成方法在 `engine_sql.rs`,异步执行方法在 `engine_exec.rs` —— 三处都写 `impl MigrationEngine { ... }` 但方法不重叠。

---

## 实施步骤

### Phase A:linq/ 拆分收尾

#### Step A1:运行测试验证

```powershell
cargo test --workspace
```

**预期**:所有测试通过(含 linq_dsl_tests、cte_syntax_tests、having_pagination_dialect_tests)。若 SQLite/Postgres/MySQL 集成测试因环境失败(无数据库),记录为环境问题,不阻塞。

#### Step A2:更新 CHANGELOG.md

在 `CHANGELOG.md` 的 `[Unreleased]` 区块顶部追加新条目(在现有 "Metadata cache" 条目之后):

```markdown
### Changed — linq.rs subdirectory split

Split `crates/macros/src/linq.rs` (2643 lines) into a `linq/` subdirectory
with 6 child modules for clearer responsibility separation:

- `ast.rs` (175 lines) — AST types (`LinqInput`, `QueryInput`, `LinqClause`,
  `HavingExprAst`)
- `parse.rs` (965 lines) — `impl Parse` + all `parse_*` functions + `ValueKind`
  + `JoinKind`
- `context.rs` (186 lines) — `LinqCtx` + `FieldKind` + `FieldRef` + field
  extraction helpers
- `compile.rs` (784 lines) — `compile_bool_expr` / `compile_expr` /
  `compile_method` / `compile_order` / `compile_having_expr` + subquery
  compilation
- `expand.rs` (412 lines) — `expand_linq` entry point + `expand_clauses` +
  `expand_join` (code generation)
- `mod.rs` (11 lines) — module declarations + `pub use expand::expand_linq`

Fixed E0027 (non-exhaustive match) in `expand_clauses`: the `LinqClause::With`
arm now destructures all 6 fields (`name`, `entity`, `param`, `body`,
`recursive`, `link`) and generates recursive CTE SQL via
`with_recursive_cte_typed` when `recursive` is true.

All internal items use `pub(crate)` visibility; only `expand_linq` is
re-exported via `pub use`. `crates/macros/src/lib.rs` unchanged — `mod linq;`
transparently resolves to `linq/mod.rs`.
```

---

### Phase B:migration.rs 子目录化

#### Step B1:创建 `migration/types.rs`(数据类型,~210 行)

**包含项目**:
- `pub struct Migration` (L15-20)
- `pub struct ModelSnapshot` (L24-28)
- `pub struct SnapshotEntityType` (L31-36)
- `pub struct SnapshotColumn` (L39-57)
- `pub enum MigrationDialect` + `impl MigrationDialect { quote, map_column_type }` (L59-160)
- `pub(crate) enum SchemaChange` (L163-206)
- `pub struct MigrationHistoryEntry` (L1260-1263)
- `pub const MIGRATION_HISTORY_TABLE: &str` (L1266)
- `pub const PRODUCT_VERSION: &str` (L1269)

**imports 头**:
```rust
//! Public data types for the migration system.
//!
//! `Migration` / `ModelSnapshot` / `SnapshotEntityType` / `SnapshotColumn`
//! are serialized forms used by the engine and store. `MigrationDialect`
//! encapsulates SQL dialect differences. `SchemaChange` is the internal
//! diff result enum.

use crate::metadata::{EntityTypeMeta, NavigationKind};
```

**可见性**:所有原有 `pub` 项保持 `pub`,`SchemaChange` 保持 `pub(crate)`。

#### Step B2:创建 `migration/diff.rs`(diff 辅助函数,~170 行)

**包含项目**:
- `fn fk_target(col: &SnapshotColumn) -> Option<(String, String)>` (L443)
- `fn index_name(table: &str, column: &str) -> String` (L454) — **注意**:这是自由函数,与 `MigrationEngine::index_name` 方法不同
- `fn fk_reference_for_property(et: &EntityTypeMeta, field: &str) -> (Option<String>, Option<String>)` (L458)
- `fn diff_foreign_keys(table: &str, old: &SnapshotColumn, new: &SnapshotColumn) -> Vec<SchemaChange>` (L480)
- `fn columns_structurally_equal(a: &SnapshotColumn, b: &SnapshotColumn) -> bool` (L550)
- `fn diff_indexes(table: &str, old: &SnapshotColumn, new: &SnapshotColumn) -> Vec<SchemaChange>` (L565)
- `pub(crate) enum IndexKind` (L590)
- `fn index_kind(col: &SnapshotColumn) -> IndexKind` (L596)

**imports 头**:
```rust
//! Diff helper functions for comparing model snapshots.
//!
//! These free functions detect schema changes between old and new
//! `SnapshotColumn` values: foreign-key additions/removals, column type
//! changes, and index additions/removals.

use crate::metadata::{EntityTypeMeta, NavigationKind};

use super::types::{SchemaChange, SnapshotColumn};
```

**可见性**:所有函数 `pub(crate)`,供 engine.rs 调用。

#### Step B3:创建 `migration/engine.rs`(引擎类型 + 第一 impl 块,~250 行)

**包含项目**:
- `pub struct MigrationEngine { dialect: MigrationDialect }` (L210-212)
- `impl MigrationEngine` 第一块(L214-442):
  - `pub fn new(dialect: MigrationDialect) -> Self`
  - `pub fn generate(&self, name: &str, current: &[EntityTypeMeta], previous_snapshot: &Option<ModelSnapshot>) -> EFResult<Migration>`
  - `pub fn create_snapshot(&self, migration_id: &str, entity_types: &[EntityTypeMeta]) -> ModelSnapshot`
  - `fn initial_create(&self, current: &ModelSnapshot) -> Vec<SchemaChange>`
  - `fn diff(&self, old: &ModelSnapshot, new: &ModelSnapshot) -> Vec<SchemaChange>`
  - `fn append_create_table_fks(...)`
  - `fn append_create_table_indexes(...)`

**imports 头**:
```rust
//! `MigrationEngine` — diffs model snapshots and generates migration SQL.
//!
//! This module contains the engine struct, its constructor, and the
//! diff/snapshot-generation logic. SQL generation methods live in
//! `engine_sql.rs`; async execution methods live in `engine_exec.rs`.

use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;

use super::diff::{
    columns_structurally_equal, diff_foreign_keys, diff_indexes, fk_reference_for_property,
};
use super::types::{Migration, MigrationDialect, ModelSnapshot, SchemaChange, SnapshotColumn,
    SnapshotEntityType};
```

**可见性**:`MigrationEngine` struct 保持 `pub`,方法可见性保持原样。

#### Step B4:创建 `migration/engine_sql.rs`(SQL 生成 impl 块,~340 行)

**包含项目**(第二 `impl MigrationEngine` 块的 SQL 部分,L606-948):
- `fn initial_create_with_fks(&self, current: &ModelSnapshot) -> Vec<SchemaChange>`
- `pub fn foreign_key_name(table: &str, column: &str, referenced_table: &str) -> String`
- `pub fn index_name(table: &str, column: &str) -> String` — **注意**:这是方法,与 diff.rs 的同名自由函数不同
- `pub fn generate_alter_column_sql(&self, table: &str, column_name: &str, new: &SnapshotColumn) -> String`
- `fn generate_up_sql(&self, changes: &[SchemaChange]) -> String`
- `fn generate_ddl_sql(&self, changes: &[SchemaChange]) -> String`
- `fn generate_up_sql_inner(&self, changes: &[SchemaChange], record_history: bool) -> String`
- `fn generate_down_sql(&self, changes: &[SchemaChange]) -> String`

**imports 头**:
```rust
//! SQL generation methods for `MigrationEngine`.
//!
//! Translates `SchemaChange` sequences into dialect-specific DDL strings
//! (CREATE TABLE / ALTER COLUMN / DROP / etc.). Split from the main
//! engine module for readability — these methods are pure SQL string
//! builders with no I/O.

use super::types::{MigrationDialect, SchemaChange, SnapshotColumn};
```

**关键**:`impl MigrationEngine { ... }` 在本文件中再次出现,Rust 允许同一类型的 impl 块跨多个文件。

#### Step B5:创建 `migration/engine_exec.rs`(异步执行 impl 块,~300 行)

**包含项目**(第二 `impl MigrationEngine` 块的执行部分,L949-1253):
- `pub async fn ensure_history_table(...)`
- `pub async fn apply(...)`
- `pub async fn revert(...)`
- `pub async fn get_applied_migrations(...)`
- `pub async fn is_applied(...)`
- `pub async fn apply_pending(...)`
- `pub async fn revert_last(...)`
- `pub async fn revert_to_target(...)`
- `pub fn generate_script(...)`
- `pub async fn ensure_created(...)`
- `pub async fn ensure_deleted(...)`
- `pub async fn apply_seed_data(...)`

**imports 头**:
```rust
//! Async execution methods for `MigrationEngine`.
//!
//! Applies generated SQL to a database connection, tracks applied
//! migrations in `__ef_migrations_history`, and supports revert/rollback.
//! Split from the main engine module for readability — these methods
//! perform I/O against `IDatabaseProvider`.

use crate::error::EFResult;
use crate::provider::IDatabaseProvider;

use super::history::{create_migration_history_table_sql, seed_insert_sql, split_sql_statements};
use super::types::{Migration, MigrationDialect, MigrationHistoryEntry, SchemaChange,
    MIGRATION_HISTORY_TABLE, PRODUCT_VERSION};
```

#### Step B6:创建 `migration/history.rs`(历史表 SQL,~80 行)

**包含项目**:
- `fn seed_insert_sql(dialect, table, columns, gen) -> String` (L1271)
- `fn split_sql_statements(sql: &str) -> Vec<String>` (L1298)
- `pub fn create_migration_history_table_sql(dialect: MigrationDialect) -> String` (L1313)

**imports 头**:
```rust
//! SQL helpers for the `__ef_migrations_history` tracking table.
//!
//! Generates dialect-specific DDL for the history table, INSERT statements
//! for recording applied migrations, and splits multi-statement scripts
//! into executable chunks.

use crate::provider::ISqlGenerator;

use super::types::{MigrationDialect, MIGRATION_HISTORY_TABLE};
```

**可见性**:`create_migration_history_table_sql` 保持 `pub`(被 sqlite_crud_tests 引用),其余 `pub(crate)`。

#### Step B7:创建 `migration/store.rs`(文件系统 I/O,~85 行)

**包含项目**:
- `pub struct MigrationStore { root: PathBuf }` (L1358-1360)
- `impl MigrationStore`(L1362-1423):
  - `pub fn new(root: impl Into<PathBuf>) -> Self`
  - `pub fn root(&self) -> &Path`
  - `pub fn save(&self, migration: &Migration) -> EFResult<()>`
  - `pub fn load_all(&self) -> EFResult<Vec<Migration>>`
  - `pub fn load(&self, id: &str) -> EFResult<Migration>`
  - `pub fn save_snapshot(&self, snapshot: &ModelSnapshot) -> EFResult<()>`
  - `pub fn load_snapshot(&self) -> EFResult<Option<ModelSnapshot>>`

**imports 头**:
```rust
//! Filesystem-backed migration script store.
//!
//! Reads and writes migration scripts under `Migrations/{id}/{up,down}.sql`
//! and persists `model_snapshot.json` between diff runs.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::EFResult;

use super::snapshot::{parse_model_snapshot_json, snapshot_to_json};
use super::types::{Migration, ModelSnapshot};
use super::migration_io_err; // 从 mod.rs 导入(或用 super::migration_io_err)
```

**注意**:`migration_io_err` 放在 `mod.rs` 作为 `pub(crate)` 函数,供 store.rs 和 snapshot.rs 共用。

#### Step B8:创建 `migration/snapshot.rs`(JSON 序列化,~130 行)

**包含项目**:
- `pub fn parse_model_snapshot_json(text: &str) -> EFResult<Option<ModelSnapshot>>` (L1426)
- `fn snapshot_to_json(snapshot: &ModelSnapshot) -> String` (L1434)
- `fn snapshot_from_json(text: &str) -> EFResult<Option<ModelSnapshot>>` (L1484)
- `fn extract_json_string(haystack: &str, key: &str) -> Option<String>` (L1545)
- `fn extract_quoted_after_colon(s: &str) -> Option<String>` (L1551)

**imports 头**:
```rust
//! JSON serialization for `ModelSnapshot`.
//!
//! Hand-rolled JSON parser/serializer (avoids pulling in serde_json for the
//! migration subsystem). Produces a stable on-disk format for diff baselines.

use crate::error::EFResult;

use super::types::{ModelSnapshot, SnapshotColumnType, SnapshotColumn};
```

#### Step B9:创建 `migration/mod.rs`(模块声明 + 重导出,~35 行)

**内容**:
```rust
//! Migration engine — model snapshot diffing, migration generation, and
//! history tracking.
//!
//! Corresponds to EFCore's migration system. Submodules:
//! - `types` — public data types + `MigrationDialect`
//! - `diff` — snapshot diffing helpers
//! - `engine` — `MigrationEngine` struct + diff/generate logic
//! - `engine_sql` — `impl MigrationEngine` SQL generation methods
//! - `engine_exec` — `impl MigrationEngine` async execution methods
//! - `history` — `__ef_migrations_history` SQL helpers
//! - `store` — filesystem-backed `MigrationStore`
//! - `snapshot` — JSON serialization for `ModelSnapshot`

mod diff;
mod engine;
mod engine_exec;
mod engine_sql;
mod history;
mod snapshot;
mod store;
mod types;

pub use history::create_migration_history_table_sql;
pub use snapshot::parse_model_snapshot_json;
pub use store::MigrationStore;
pub use types::{
    Migration, MigrationDialect, MigrationHistoryEntry, ModelSnapshot, SnapshotColumn,
    SnapshotEntityType, MIGRATION_HISTORY_TABLE, PRODUCT_VERSION,
};
pub use engine::MigrationEngine;

// Re-exported for tests / internal use (SchemaChange is pub(crate)).
pub(crate) use types::SchemaChange;

/// Converts `std::io::Error` into `EFError::Migration`.
/// Shared by `store.rs` and `snapshot.rs`.
pub(crate) fn migration_io_err(e: std::io::Error) -> crate::error::EFError {
    crate::error::EFError::Migration(e.to_string())
}
```

#### Step B10:删除原 `migration.rs`

```powershell
Remove-Item "e:\GitCode\RF\rust-ef\crates\core\src\migration.rs"
```

**验证**:`crates/core/src/lib.rs:35` 的 `pub mod migration;` 声明对目录模式透明,自动解析到 `migration/mod.rs`,无需修改。

#### Step B11:编译验证

```powershell
cargo check --workspace
```

**预期**:0 错误。若有 imports 遗漏或可见性不足,逐个修复。

#### Step B12:测试验证

```powershell
cargo test --workspace
```

**预期**:所有已有测试通过,特别是:
- `migration_cli_tests`(Migration / MigrationDialect / MigrationEngine)
- `index_diff_tests`(MigrationDialect / MigrationEngine)
- `integration_tests`(MigrationEngine::foreign_key_name、SnapshotColumn)
- `sqlite_crud_tests`(create_migration_history_table_sql)
- `production_tests`(MigrationStore)

#### Step B13:更新 CHANGELOG.md

在 `CHANGELOG.md` 的 `[Unreleased]` 区块追加(在 Phase A 的 linq 条目之后):

```markdown
### Changed — migration.rs subdirectory split

Split `crates/core/src/migration.rs` (1449 lines) into a `migration/`
subdirectory with 9 child modules for clearer responsibility separation:

- `types.rs` (~210 lines) — `Migration` / `ModelSnapshot` /
  `SnapshotEntityType` / `SnapshotColumn` / `MigrationDialect` /
  `SchemaChange` / `MigrationHistoryEntry` + constants
- `diff.rs` (~170 lines) — snapshot diffing helpers (`fk_target`,
  `diff_foreign_keys`, `diff_indexes`, `columns_structurally_equal`)
- `engine.rs` (~250 lines) — `MigrationEngine` struct + diff/generate
  impl block (`new`, `generate`, `create_snapshot`, `diff`)
- `engine_sql.rs` (~340 lines) — `impl MigrationEngine` SQL generation
  methods (`generate_up_sql`, `generate_down_sql`,
  `generate_alter_column_sql`, `foreign_key_name`, `index_name`)
- `engine_exec.rs` (~300 lines) — `impl MigrationEngine` async execution
  methods (`apply`, `revert`, `apply_pending`, `ensure_history_table`,
  `ensure_created`, `apply_seed_data`)
- `history.rs` (~80 lines) — `__ef_migrations_history` SQL helpers
- `store.rs` (~85 lines) — `MigrationStore` filesystem I/O
- `snapshot.rs` (~130 lines) — JSON serialization for `ModelSnapshot`
- `mod.rs` (~35 lines) — module declarations + `pub use` re-exports +
  shared `migration_io_err` helper

The `MigrationEngine` impl block is split across 3 files (engine.rs,
engine_sql.rs, engine_exec.rs) — Rust allows multiple `impl` blocks for
the same type. All public API paths (`rust_ef::migration::*`) are
preserved via `pub use` re-exports in `mod.rs`. No behavioral changes.
```

---

## 假设与决策

1. **不调整 migration.rs 原有逻辑**:本次仅做物理拆分,不重构算法、不优化性能、不改变行为。所有 SQL 生成、diff 逻辑、执行流程保持原样。

2. **`MigrationEngine` impl 块跨 3 文件**:Rust 允许同一类型的 `impl` 块出现在多个文件中。engine.rs(构造 + diff)、engine_sql.rs(SQL 生成)、engine_exec.rs(异步执行)各自有 `impl MigrationEngine { ... }` 但方法不重叠。这避免了单文件 850 行的堆积,符合用户"避免大量逻辑堆积"的偏好。

3. **`migration_io_err` 放在 mod.rs**:该函数被 store.rs 和 snapshot.rs 共用,放在 mod.rs 作为 `pub(crate)` 函数,两个子模块通过 `super::migration_io_err` 引用。

4. **`SchemaChange` 保持 `pub(crate)`**:原文件中即为 `pub(crate)`,在 types.rs 中保持不变,mod.rs 通过 `pub(crate) use types::SchemaChange;` 重导出供 crate 内其他模块(若有)使用。

5. **公共 API 路径完全保持**:`rust_ef::migration::{Migration, MigrationDialect, MigrationEngine, MigrationStore, SnapshotColumn, MigrationHistoryEntry, ModelSnapshot, SnapshotEntityType, MIGRATION_HISTORY_TABLE, PRODUCT_VERSION, create_migration_history_table_sql, parse_model_snapshot_json}` 全部通过 mod.rs 的 `pub use` 重导出,外部引用路径不变。

6. **不修改 lib.rs**:`pub mod migration;` 声明对目录模式透明,无需改动。

7. **imports 最小化**:每个子模块只导入自身需要的项,避免 unused imports 警告。如遇警告,按编译器提示精确调整。

8. **保留原有注释**:section 分隔注释(`// ---`)和 doc comment 保留在对应子模块中。

---

## 验证步骤

| 步骤 | 验证内容 | 预期结果 |
|------|---------|---------|
| Step A1 | `cargo test --workspace` | 所有测试通过(或仅环境相关失败) |
| Step A2 | CHANGELOG.md 检查 | linq 拆分条目完整、准确 |
| Step B1-B9 | 9 个子模块文件创建完成 | 文件存在,imports 正确 |
| Step B10 | `migration.rs` 删除 | 文件不存在,`pub mod migration;` 解析成功 |
| Step B11 | `cargo check --workspace` | 0 错误,0 警告(或仅 pre-existing 警告) |
| Step B12 | `cargo test --workspace` | 所有测试通过(含 migration_cli_tests、index_diff_tests、integration_tests、sqlite_crud_tests、production_tests) |
| Step B13 | CHANGELOG.md 检查 | migration 拆分条目完整、准确 |

---

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| imports 遗漏导致 E0432 | 每个子模块创建后立即 `cargo check`,逐个修复 |
| 跨模块可见性不足(E0603) | 统一使用 `pub(crate)`,编译器会提示需提升的项 |
| `impl MigrationEngine` 跨文件导致方法重复定义(E0201) | 三个 impl 块的方法集已严格划分,无重叠;编译器会立即报错 |
| `index_name` 同名混淆(diff.rs 自由函数 vs engine_sql.rs 方法) | 自由函数用 `diff::index_name(...)`,方法用 `MigrationEngine::index_name(...)` 或 `self.index_name(...)`,语义清晰 |
| 删除 migration.rs 后路径冲突 | 确保先创建 `migration/mod.rs` 再删除 `migration.rs` |
| 公共 API 路径断裂 | mod.rs `pub use` 列表已对照 grep 结果完整覆盖所有外部引用项 |
| 历史 SQL 辅助函数被 engine_exec 和 tests 引用 | `create_migration_history_table_sql` 保持 `pub`,`seed_insert_sql`/`split_sql_statements` 提升为 `pub(crate)` |
