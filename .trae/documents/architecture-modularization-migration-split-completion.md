# 架构模块化:migration.rs 子目录拆分(收尾阶段)

## 概述

继续推进 `crates/core/src/migration.rs`(1560 行,单文件堆积)的子目录化拆分。
前序会话已完成 9 个子模块中的 5 个(types/diff/engine/engine_sql/engine_exec),
本计划完成剩余 4 个文件 + 删除原文件 + 编译/测试验证 + CHANGELOG 更新。

## 当前状态分析

### 已完成(5/9 文件,已通过 cargo check 编译)

| 文件 | 行数 | 职责 | 关键可见性调整 |
|------|------|------|----------------|
| `migration/types.rs` | 216 | 公共数据类型 + `SchemaChange` 内部枚举 + 常量 | `SchemaChange` 为 `pub(crate)` |
| `migration/diff.rs` | 179 | 快照差异比较的自由函数 + `IndexKind` | 所有函数 `pub(crate)` |
| `migration/engine.rs` | 261 | `MigrationEngine` 结构体 + 构造 + diff 逻辑 | `dialect` 字段、`initial_create`/`diff`/`append_create_table_fks`/`append_create_table_indexes` 升为 `pub(crate)` |
| `migration/engine_sql.rs` | 361 | 第二个 `impl MigrationEngine`(SQL 生成) | `initial_create_with_fks`/`generate_up_sql`/`generate_ddl_sql`/`generate_down_sql` 为 `pub(crate)` |
| `migration/engine_exec.rs` | 327 | 第三个 `impl MigrationEngine`(异步执行) | 全部 `pub` 方法保留 |

### 已验证的依赖关系

`engine_exec.rs` 已经声明:
```rust
use super::history::{create_migration_history_table_sql, seed_insert_sql, split_sql_statements};
use super::types::{Migration, MigrationHistoryEntry, MIGRATION_HISTORY_TABLE};
```

因此 `history.rs` 中 `seed_insert_sql` 与 `split_sql_statements` 必须为 `pub(crate)`,
`create_migration_history_table_sql` 必须为 `pub`(外部测试 `sqlite_crud_tests.rs` 直接使用)。

### 公共 API 约束(12 项,必须从 `mod.rs` 通过 `pub use` 再导出)

通过 grep `rust_ef::migration::` / `crate::migration::` 扫描所有调用方确认:

| 公共项 | 类型 | 外部调用方 |
|--------|------|-----------|
| `Migration` | struct | `migration_cli_tests`, `production_tests`(via MigrationStore) |
| `MigrationDialect` | enum | 几乎所有测试 + 3 个 provider crate |
| `MigrationEngine` | struct + 方法 | `db_context.rs`, `common/mod.rs`, `production_tests`, `sqlite_crud_tests`, `index_diff_tests`, `advanced_tests` |
| `MigrationEngine::foreign_key_name` | pub 方法 | `integration_tests` |
| `MigrationEngine::index_name` | pub 方法 | (保留为 pub,与 `diff::index_name` 同名但不同路径) |
| `MigrationEngine::generate_alter_column_sql` | pub 方法 | (保留为 pub) |
| `MigrationHistoryEntry` | struct | (内部使用,保留 pub 以防外部依赖) |
| `MigrationStore` | struct | `production_tests` |
| `ModelSnapshot` | struct | (MigrationStore::save_snapshot 签名需要) |
| `SnapshotEntityType` | struct | (ModelSnapshot 字段类型,保留 pub) |
| `SnapshotColumn` | struct | `extended_types_tests`, `integration_tests` |
| `MIGRATION_HISTORY_TABLE` | const | (内部使用) |
| `PRODUCT_VERSION` | const | (内部使用) |
| `create_migration_history_table_sql` | pub fn | `sqlite_crud_tests` |
| `parse_model_snapshot_json` | pub fn | (原文件标记为 pub,保留) |

### 原始 migration.rs 剩余未拆分内容(L1255-1560)

- L1255-1269: `MigrationHistoryEntry` + `MIGRATION_HISTORY_TABLE` + `PRODUCT_VERSION`(已在 types.rs 中,可删除)
- L1271-1295: `seed_insert_sql` → history.rs
- L1297-1310: `split_sql_statements` → history.rs
- L1312-1339: `create_migration_history_table_sql` → history.rs
- L1341-1423: `MigrationStore` 结构体 + impl → store.rs
- L1425-1428: `parse_model_snapshot_json` → snapshot.rs
- L1430-1432: `migration_io_err` → mod.rs(共享)
- L1434-1482: `snapshot_to_json` → snapshot.rs
- L1484-1543: `snapshot_from_json` → snapshot.rs(私有)
- L1545-1549: `extract_json_string` → snapshot.rs(私有)
- L1551-1560: `extract_quoted_after_colon` → snapshot.rs(私有)

## 实施变更(4 个新文件 + 1 删除 + 验证 + CHANGELOG)

### Step B6:创建 `migration/history.rs`(历史表 SQL,~75 行)

**包含项目**(来自原 migration.rs L1271-1339):
- `pub(crate) fn seed_insert_sql(dialect, table, columns, gen) -> String`
- `pub(crate) fn split_sql_statements(sql: &str) -> Vec<String>`
- `pub fn create_migration_history_table_sql(dialect) -> String`

**imports 头**:
```rust
//! Migration history table SQL helpers.
//!
//! DDL for `__ef_migrations_history` and seed-insert helpers used by the
//! async execution methods in `engine_exec.rs`.

use crate::provider::ISqlGenerator;
use super::types::{MigrationDialect, MIGRATION_HISTORY_TABLE};
```

**可见性策略**:
- `seed_insert_sql` / `split_sql_statements`:从 `fn` 升为 `pub(crate) fn`(engine_exec.rs 跨模块调用)
- `create_migration_history_table_sql`:保持 `pub fn`(外部测试直接调用)

### Step B7:创建 `migration/store.rs`(文件系统迁移存储,~95 行)

**包含项目**(来自原 migration.rs L1341-1423):
- `pub struct MigrationStore { root: PathBuf }`
- `impl MigrationStore`:`new` / `root` / `save` / `load_all` / `load` / `save_snapshot` / `load_snapshot`

**imports 头**:
```rust
//! Filesystem migration store (CLI / project migrations folder).
//!
//! Reads and writes migration scripts in `{root}/{id}/up.sql` + `down.sql`
//! layout, plus a `model_snapshot.json` baseline file.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::EFResult;

use super::snapshot::{parse_model_snapshot_json, snapshot_to_json};
use super::types::{Migration, ModelSnapshot};
```

**可见性策略**:全部保持 `pub`(原文件已是 pub)。
**`migration_io_err` 引用**:从 mod.rs 引入,通过 `super::migration_io_err` 访问。

### Step B8:创建 `migration/snapshot.rs`(JSON 序列化,~130 行)

**包含项目**(来自原 migration.rs L1425-1560):
- `pub fn parse_model_snapshot_json(text: &str) -> EFResult<Option<ModelSnapshot>>`(外部 pub)
- `pub(crate) fn snapshot_to_json(snapshot: &ModelSnapshot) -> String`(store.rs 调用)
- `fn snapshot_from_json(text: &str) -> EFResult<Option<ModelSnapshot>>`(私有,仅 parse_model_snapshot_json 调用)
- `fn extract_json_string(haystack, key) -> Option<String>`(私有)
- `fn extract_quoted_after_colon(s: &str) -> Option<String>`(私有)

**imports 头**:
```rust
//! Model snapshot JSON serialization.
//!
//! Minimal hand-rolled JSON writer/parser for `ModelSnapshot`. Used by
//! `MigrationStore` to persist the diff baseline. The format is stable and
//! human-readable.

use crate::error::EFResult;

use super::migration_io_err;
use super::types::{ModelSnapshot, SnapshotColumn, SnapshotEntityType};
```

**可见性策略**:
- `parse_model_snapshot_json`:保持 `pub`(外部 API 一部分)
- `snapshot_to_json`:从 `fn` 升为 `pub(crate) fn`(store.rs 跨模块调用)
- `snapshot_from_json` / `extract_json_string` / `extract_quoted_after_colon`:保持私有 `fn`

### Step B9:创建 `migration/mod.rs`(模块声明 + 再导出 + 共享 helper,~40 行)

**内容**:
```rust
//! Migration engine — model snapshot diffing, migration generation, and
//! history tracking.
//!
//! Corresponds to EFCore's migration system. Split into submodules for
//! readability:
//! - `types`: public data types + `SchemaChange` internal enum + constants
//! - `diff`: snapshot diff helper free functions
//! - `engine`: `MigrationEngine` struct + constructor + diff logic
//! - `engine_sql`: SQL generation `impl MigrationEngine` block
//! - `engine_exec`: async execution `impl MigrationEngine` block
//! - `history`: `__ef_migrations_history` table DDL + seed insert SQL
//! - `store`: filesystem migration script store (CLI)
//! - `snapshot`: model snapshot JSON I/O

mod diff;
mod engine;
mod engine_exec;
mod engine_sql;
mod history;
mod snapshot;
mod store;
mod types;

pub use engine::MigrationEngine;
pub use history::create_migration_history_table_sql;
pub use snapshot::parse_model_snapshot_json;
pub use store::MigrationStore;
pub use types::{
    Migration, MigrationDialect, MigrationHistoryEntry, MIGRATION_HISTORY_TABLE, ModelSnapshot,
    PRODUCT_VERSION, SnapshotColumn, SnapshotEntityType,
};

/// Converts `std::io::Error` to `EFError::Migration`. Shared by `store.rs`
/// and `snapshot.rs`.
pub(crate) fn migration_io_err(e: std::io::Error) -> crate::error::EFError {
    crate::error::EFError::Migration(e.to_string())
}
```

### Step B10:删除原始 `migration.rs`

使用 DeleteFile 工具删除 `e:\GitCode\RF\rust-ef\crates\core\src\migration.rs`。
此时 `lib.rs:35` 的 `pub mod migration;` 会自动解析到 `migration/mod.rs`(Rust 目录模块模式)。

### Step B11:`cargo check --workspace`

**验证标准**:编译通过,无错误。允许保留警告(如 `select.rs:15` 的 unused import 警告,与本任务无关)。

**预期风险点**:
1. `MigrationStore` 中 `use std::fs; use std::path::{Path, PathBuf};` 必须迁移到 store.rs(原文件中部声明)
2. `migration_io_err` 在 mod.rs 中定义后,store.rs 通过 `super::migration_io_err` 引用,snapshot.rs 同样
3. `index_name` 自由函数(diff.rs)与 `MigrationEngine::index_name` 方法(engine_sql.rs)路径不同,不会冲突

### Step B12:`cargo test --workspace --no-fail-fast`

**验证标准**:
- 关键测试通过:`migration_cli_tests`(13)、`index_diff_tests`(10)、`advanced_tests`(6)、`integration_tests`(16)、`extended_types_tests`(6)、`production_tests`(包含 MigrationStore)
- 总计预期 >200 测试通过
- DB-dependent 测试(mysql_crud_tests / postgres_crud_tests)若环境无数据库则允许失败/挂起,通过 StopCommand 处理

### Step B13:更新 CHANGELOG.md

在 `[Unreleased]` 段尾(`### Changed — linq.rs subdirectory split` 之后,`---` 分隔符之前)追加:

```markdown
### Changed — migration.rs subdirectory split

Split `crates/core/src/migration.rs` (1560 lines) into a `migration/`
subdirectory with 9 child modules for clearer responsibility separation:

- `types.rs` (216 lines) — `Migration` / `ModelSnapshot` / `SnapshotEntityType`
  / `SnapshotColumn` / `MigrationDialect` + impl / `SchemaChange` (pub(crate))
  / `MigrationHistoryEntry` / `MIGRATION_HISTORY_TABLE` / `PRODUCT_VERSION`
- `diff.rs` (179 lines) — snapshot diff free functions (`fk_target`,
  `index_name`, `fk_reference_for_property`, `diff_foreign_keys`,
  `columns_structurally_equal`, `diff_indexes`, `IndexKind`, `index_kind`)
- `engine.rs` (261 lines) — `MigrationEngine` struct + constructor +
  `generate` / `create_snapshot` / `initial_create` / `diff` /
  `append_create_table_fks` / `append_create_table_indexes`
- `engine_sql.rs` (361 lines) — second `impl MigrationEngine` block (SQL
  generation): `initial_create_with_fks` / `foreign_key_name` / `index_name`
  / `generate_alter_column_sql` / `generate_up_sql` / `generate_ddl_sql`
  / `generate_down_sql`
- `engine_exec.rs` (327 lines) — third `impl MigrationEngine` block (async
  execution): `ensure_history_table` / `apply` / `revert` /
  `get_applied_migrations` / `is_applied` / `apply_pending` / `revert_last`
  / `revert_to_target` / `generate_script` / `ensure_created` /
  `ensure_deleted` / `apply_seed_data`
- `history.rs` (~75 lines) — `seed_insert_sql` / `split_sql_statements`
  / `create_migration_history_table_sql`
- `store.rs` (~95 lines) — `MigrationStore` filesystem store
- `snapshot.rs` (~130 lines) — `parse_model_snapshot_json` /
  `snapshot_to_json` / `snapshot_from_json` + JSON helpers
- `mod.rs` (~40 lines) — module declarations + `pub use` re-exports +
  shared `migration_io_err` helper

Internal visibility strategy: private methods escalated to `pub(crate)`
for cross-module calls (`initial_create`, `diff`,
`append_create_table_fks`, `append_create_table_indexes`,
`initial_create_with_fks`, `generate_up_sql`, `generate_ddl_sql`,
`generate_down_sql`, `seed_insert_sql`, `split_sql_statements`,
`snapshot_to_json`). The `MigrationEngine.dialect` field changed from
private to `pub(crate)` so `engine_sql.rs` and `engine_exec.rs` can
access it. `migration_io_err` is defined in `mod.rs` as `pub(crate)` and
shared by `store.rs` and `snapshot.rs`.

All 12 public items are re-exported from `mod.rs` via `pub use`,
preserving the `rust_ef::migration::*` public API surface.
`crates/core/src/lib.rs:35` `pub mod migration;` transparently resolves
to `migration/mod.rs` — no lib.rs change needed.
```

## 假设与决策

### 假设
1. 已存在的 5 个文件(types/diff/engine/engine_sql/engine_exec)内容正确,无需重新生成
2. 原始 `migration.rs` L1255-1560 的内容与 Read 工具读取的一致
3. `lib.rs:35` `pub mod migration;` 在目录模式下自动解析到 `migration/mod.rs`,无需修改 lib.rs

### 决策
1. **`migration_io_err` 放在 mod.rs** 而非 snapshot.rs:因为 store.rs 和 snapshot.rs 都需要,放在公共父模块更清晰
2. **`pub(crate) fn` 升级路径** 严格匹配跨模块调用需求:不升级不需要跨模块访问的私有函数(如 `snapshot_from_json` / `extract_json_string` / `extract_quoted_after_colon` 保持私有)
3. **`pub use` 使用模块路径聚合**:`pub use types::{...}` 而非逐项 `pub use types::Migration;`,减少 mod.rs 行数
4. **不修改 lib.rs**:`pub mod migration;` 在文件模式和目录模式下都工作,Rust 模块系统透明处理

## 风险与缓解

| 风险 | 缓解措施 |
|------|---------|
| `migration_io_err` 在 mod.rs 中定义,但 store.rs/snapshot.rs 使用 `super::migration_io_err` 失败 | Rust 允许在父模块定义函数,子模块通过 `super::` 访问,语法已验证 |
| `index_name` 同名冲突(diff.rs 自由函数 vs engine_sql.rs 方法) | 路径不同:`diff::index_name` 自由函数 vs `MigrationEngine::index_name` 关联函数,无冲突 |
| 删除 migration.rs 后 lib.rs 找不到模块 | `pub mod migration;` 在目录模式下自动解析到 `migration/mod.rs`,Rust 标准行为 |
| `MigrationStore` 中 `use std::fs; use std::path::{Path, PathBuf};` 原在文件中部声明 | 迁移到 store.rs 顶部 imports 区,Rust 允许 use 在任意位置但惯例在顶部 |
| cargo test 因 DB 环境缺失挂起(mysql/postgres crud tests) | 使用 `--no-fail-fast` + 后台运行 + StopCommand 处理挂起的测试,记录为环境问题 |

## 验证步骤

1. **Step B6-B9 完成 4 个文件创建** → 验证:文件存在且内容完整
2. **Step B10 删除 migration.rs** → 验证:Glob `migration*.rs` 仅返回目录
3. **Step B11 cargo check** → 验证:编译通过 0 errors(允许 warnings)
4. **Step B12 cargo test** → 验证:关键测试套件全通过(migration_cli_tests 13/13, index_diff_tests 10/10, advanced_tests 6/6, integration_tests 16/16, extended_types_tests 6/6, production_tests 含 MigrationStore 通过)
5. **Step B13 CHANGELOG 更新** → 验证:新条目出现在 `[Unreleased]` 段尾,`---` 分隔符之前
