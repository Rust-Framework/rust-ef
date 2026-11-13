# 自动升级流程（下一次迭代加字段）

结论：**可空列 / 带默认值的列可以全自动生成并应用**；完整链路需要三步（导出当前快照 → 生成迁移 → 应用迁移）。框架负责 diff 与 DDL，业务不再手写 `ALTER TABLE`。

## 1. 应用内导出当前快照

Rust 没有运行时反射，独立 CLI 看不到应用通过 `inventory` 注册的实体，所以**快照必须由应用自己导出**。加一个很小的子命令或 `src/bin/snapshot.rs`：

```rust
use rust_ef::migration::{save_registered_snapshot, MigrationDialect};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    save_registered_snapshot(
        "current_snapshot.json",
        None,                        // 默认 context；keyed 库传 Some("logs")
        MigrationDialect::MySql,     // 与目标库一致（sqlite/postgres/mysql）
    )?;
    Ok(())
}
```

`save_registered_snapshot` 会遍历所有 `#[derive(EntityType)]` + `#[entity(T)]` 配置，生成与目标方言一致的快照（含索引/约束/默认值/注释/计算列）。

## 2. 生成迁移

```bash
cargo run --bin snapshot
rust-ef migration add AddOrderStock \
  --snapshot current_snapshot.json \
  --dir Migrations \
  --dialect mysql
```

- `Migrations/model_snapshot.json` 是**上一次**的基线；第一次运行时不存在，按全量建表生成。
- 生成后 CLI 会把 `current_snapshot.json` 写入 `Migrations/model_snapshot.json`，供下次 diff。
- 产物：`Migrations/AddOrderStock/up.sql`、`down.sql`，纳入版本控制并做 code review。

## 3. 应用迁移

生产启动时（EF Core `Database.Migrate()` 等价）：

```rust
let ctx = DbContext::from_options(&options)?;
let applied = ctx.migrate("Migrations").await?;
tracing::info!(applied, "database migrations applied");
```

或部署流水线执行：

```bash
rust-ef migration apply --connection "$DATABASE_URL" --dir Migrations
```

- 已应用记录在 `__ef_migrations_history`；重复执行是 no-op。
- PostgreSQL / SQLite：整个迁移脚本（含历史行）在**一个事务**里执行，失败自动回滚。
- MySQL：DDL 隐式提交，**无法整体回滚**；失败后迁移不会被记录，已提交的语句会保留，需要人工修复到一致状态再重试。
  引擎不伪造原子性：失败时返回的错误（`source` 为 `PartialMigrationFailure`）与错误消息都会逐条列出
  `executed`（已提交，按执行顺序）、`failed`（失败语句）和 `not_executed`（未执行）语句，
  据此可确定性地清理已创建对象或补跑剩余语句。同一语义适用于 `apply` / `revert` / `bootstrap`。
- 并发发布：不要多个实例同时迁移；在发布流程里只让一个 job 执行 `migrate`/`apply`。
- 迁移 id 由 CLI 自动加 Unix 时间戳前缀，`load_all` 按 id 排序即创建顺序；`MigrationStore` 会拒绝 `../`、路径分隔符等不安全 id。

## 4. 各类变更的自动化程度

| 变更 | 自动生成 | 应用后效果 |
|---|---|---|
| 新增可空列 | ✅ `ADD COLUMN ... NULL` | 旧行 NULL |
| 新增带默认值列 | ✅ `ADD COLUMN ... DEFAULT ...` | 旧行取默认值 |
| 新增 NOT NULL 且无默认值 | ⚠️ 生成警告注释 + SQL | 非空表会失败；补 `has_default(...)` 或先回填 |
| 新增索引 / 唯一约束 | ✅ `CREATE [UNIQUE] INDEX` | 立即生效 |
| 新增 CHECK | ✅ PG/MySQL 建表内联或 `ADD CONSTRAINT` | SQLite 对既有表输出 warning（需重建表；运行时差分会自动重建，见第 7 节） |
| 新增 FK / 复合 FK | ✅ PG/MySQL `ADD CONSTRAINT` | SQLite 对既有表输出 warning（需重建表；运行时差分会自动重建，见第 7 节） |
| 修改列类型/精度/默认值/注释 | ✅ `ALTER COLUMN` / `MODIFY COLUMN` | SQLite 不支持类型变更（warning，需重建表；运行时差分会自动重建） |
| 删除列/表 | ⚠️ 生成警告注释 + SQL | **不可逆、数据丢失**；列重命名会被识别为删除+新增 |
| 数据回填 / 重命名 | ❌ | 写自定义 SQL 迁移（属于业务数据迁移） |

## 5. SQLite 的升级边界

SQLite 的 `ALTER TABLE` 能力有限：

- `ADD COLUMN` 支持，但 `NOT NULL` 必须带非空默认值；不能加 `PRIMARY KEY` / `UNIQUE` 列约束（`UNIQUE` 用 `CREATE UNIQUE INDEX` 代替，rust-ef 会这样做）。
- 既有表不能 `ADD/DROP CHECK`、`ADD/DROP FOREIGN KEY`；rust-ef 输出 `-- WARNING` 注释而不是非法 SQL。
- 修改列类型、删除带索引/约束的列需要重建表。

需要完整约束升级时，优先 PostgreSQL/MySQL；SQLite 场景用第 7 节的运行时差分（会自动重建表），或把 schema 变更安排在开发期重建。

## 7. 运行时差分（已装库升级，2.3.0+）

第 1–3 节是「导出快照 → 生成迁移文件 → 应用」的版本化流程。2.3.0 起还有一条**运行时**路径：
不生成迁移文件，而是把模型与**已装库的真实 schema**（provider 内省结果）比较，直接生成并应用 ALTER。
它适合安装器/启动自检：模型是唯一真相，已装库自动对齐。

```rust
use rust_ef::migration::{MigrationDialect, MigrationEngine};

let engine = MigrationEngine::new(MigrationDialect::Sqlite);
let model = ctx.model().build();                 // 或 create_snapshot(...) 得到的 ModelSnapshot
let live = provider.introspect().await?;         // 框架内省（EF-5，返回 Vec<rust_ef::schema::DbTable>）

let diff = engine.diff_metas(&model, &live);     // 或 engine.diff(&snapshot, &live)
if !diff.is_empty() {
    tracing::info!(report = %diff.report(), "runtime schema upgrade"); // 升级报告（含重建表清单/warnings）
    engine.apply_diff(&provider, &diff).await?;  // 原子应用
}
```

### 7.1 `MigrationDiff` 输出

| 字段 | 含义 |
|---|---|
| `statements: Vec<String>` | 按依赖顺序排好的可执行 SQL（建表 → SQLite 重建 → 加列 → 建索引 → 加约束） |
| `warnings: Vec<DiffWarning>` | 结构化警告：`severity`(info/warning) + `kind` + `table` + `message`，可直接写日志/升级报告 |
| `rebuilds: Vec<TableRebuild>` | SQLite 将重建的表：拷贝列、新列、无默认值的 NOT NULL 新列、触发器是否保留 |

辅助方法：`is_empty()`、`up_sql()`、`report()`（人类可读升级报告）、`rebuild_count()`、
`into_migration(id, dialect)`（转成带 `__ef_migrations_history` 记录的版本化 `Migration`，供已有 `apply` 流程使用）。

**为什么 `diff` 不直接返回 `Migration`**：`Migration { id, description, up_sql, down_sql }` 是公开结构体，
外部代码大量用结构体字面量构造；在 2.3.0（minor）给它加 `warnings` 字段是破坏性变更。运行时差分也没有
版本化 id 和可靠 Down，因此用独立的 `MigrationDiff` 类型（只想走版本化流程时用 `into_migration`）。

### 7.2 warnings 语义（不是「忽略」，是「明确报告」）

| kind | 含义 | 业务动作 |
|---|---|---|
| `skipped_removal` | 库里有、模型里没有的表/列/索引/约束，**不会被删除** | 记录；确需删除时写版本化迁移 |
| `constraint_not_introspected` | provider 无法内省 CHECK 约束（`DbTable.checks == None`） | 升级报告提示；不要假定约束已一致 |
| `not_null_column_without_default` | 新增 NOT NULL 无默认列 | 加 `has_default` 或先回填 |
| `index_definition_mismatch` | 同名索引定义不同 | 版本化迁移 drop + create |
| `unsupported_change` | 方言不支持（MySQL 部分非唯一索引、SQLite 以外的 INCLUDE 等） | 按提示调整模型或改用支持的方言 |
| `manual_action` | 需要人工（PG 生成列改表达式、SQLite 重建会丢未声明列、触发器无法内省等） | 人工处理后再升级 |
| `table_rebuild` | 保留值（重建信息通过 `rebuilds` 提供） | — |

### 7.3 安全策略

- **只自动应用加法**：建表/加列/索引/唯一索引/CHECK/FK；删除类变更一律只告警，绝不自动执行。
- **不猜测未知内省**：`DbTable.checks/triggers` 为 `None` 表示「provider 没法内省」，此时不会重复添加约束，
  而是给出 `constraint_not_introspected`。
- **SQLite 重建表**：SQLite 不能 `ALTER` 约束/列类型，运行时差分用「影子表 + 拷数据 + 换名」重建
  （`__ef_rebuild_<table>`）：公共列 `INSERT ... SELECT` 复制、新列走默认值、重建模型索引并保留既有
  非模型索引、重建触发器。执行由 `apply_diff` 负责：**单事务** + 事务外 `PRAGMA foreign_keys=OFF` +
  commit 前 `PRAGMA foreign_key_check` + 任意失败整体回滚（不留影子表、不丢数据），结束后恢复原 PRAGMA。
- **预检**：重建表且有「新 NOT NULL 无默认列」而表非空时，`apply_diff` 在动库前返回 typed error。
- **拒绝丢数据**：库中存在模型中已删除的列时，拒绝自动重建（`manual_action`），因为重建会丢这些列的数据。

### 7.4 与版本化迁移的选择

| 场景 | 用法 |
|---|---|
| 安装器/升级自检、明确只做加法（加列/索引/约束） | `diff` + `apply_diff`（无需迁移文件） |
| 删除列/表、重命名、数据回填、需要历史记录 | 版本化迁移（1–3 节）；可先 `diff` 生成 SQL 参考 |
| 需要 `__ef_migrations_history` 记录 | `diff.into_migration("Upgrade_2026", dialect)` 后走 `apply` |

> 运行时差分不写 `__ef_migrations_history`；`apply_diff` 重复执行需要重新 diff（天然幂等：diff 为空即无操作）。


## 6. 最小验收

```bash
cargo test -p rust-ef --all-features --test migration_automation_tests
```

覆盖：

- 加列（默认值 + 可空自定义类型）→ 生成 `ALTER TABLE` → 应用后旧行取默认值；
- 迁移脚本失败在 SQLite 上原子回滚；
- `DbContext::migrate` 首次应用返回 1、重复执行返回 0。
