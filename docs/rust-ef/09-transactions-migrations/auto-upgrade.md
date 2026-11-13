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
| 新增 CHECK | ✅ PG/MySQL 建表内联或 `ADD CONSTRAINT` | SQLite 对既有表输出 warning（需重建表） |
| 新增 FK / 复合 FK | ✅ PG/MySQL `ADD CONSTRAINT` | SQLite 对既有表输出 warning（需重建表） |
| 修改列类型/精度/默认值/注释 | ✅ `ALTER COLUMN` / `MODIFY COLUMN` | SQLite 不支持类型变更（warning，需重建表） |
| 删除列/表 | ⚠️ 生成警告注释 + SQL | **不可逆、数据丢失**；列重命名会被识别为删除+新增 |
| 数据回填 / 重命名 | ❌ | 写自定义 SQL 迁移（属于业务数据迁移） |

## 5. SQLite 的升级边界

SQLite 的 `ALTER TABLE` 能力有限：

- `ADD COLUMN` 支持，但 `NOT NULL` 必须带非空默认值；不能加 `PRIMARY KEY` / `UNIQUE` 列约束（`UNIQUE` 用 `CREATE UNIQUE INDEX` 代替，rust-ef 会这样做）。
- 既有表不能 `ADD/DROP CHECK`、`ADD/DROP FOREIGN KEY`；rust-ef 输出 `-- WARNING` 注释而不是非法 SQL。
- 修改列类型、删除带索引/约束的列需要重建表。

需要完整约束升级时，优先 PostgreSQL/MySQL；SQLite 场景把 schema 变更安排在开发期重建。

## 6. 最小验收

```bash
cargo test -p rust-ef --all-features --test migration_automation_tests
```

覆盖：

- 加列（默认值 + 可空自定义类型）→ 生成 `ALTER TABLE` → 应用后旧行取默认值；
- 迁移脚本失败在 SQLite 上原子回滚；
- `DbContext::migrate` 首次应用返回 1、重复执行返回 0。
