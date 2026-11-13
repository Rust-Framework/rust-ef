# 迁移引擎与种子数据

## MigrationEngine

```rust
use rust_ef::migration::{MigrationEngine, MigrationDialect};

let engine = MigrationEngine::new(MigrationDialect::Sqlite);
let migration = engine.generate(
    "InitialCreate",
    &[Blog::entity_meta(), Post::entity_meta()],
    &None,  // 无前一次快照
)?;

println!("Up SQL:\n{}", migration.up_sql);
println!("Down SQL:\n{}", migration.down_sql);
```

差分覆盖：表/列增删、列类型/精度/定长/排序规则/注释、默认值、计算列、单列与复合索引（含唯一、部分、表达式索引与 PostgreSQL INCLUDE）、标量与复合外键、CHECK 约束、MySQL 表选项、表注释。索引和 CHECK 的定义会保存在快照中（`model_snapshot.json`），增删或修改会生成对应的 `CREATE` / `DROP` 语句；`Down` 会用旧定义重建被删除的索引与约束。

## 种子数据

```rust
ctx.model().entity::<Blog>().has_data(&[
    Blog { blog_id: 1, url: "https://seed.example".into(), rating: 5 },
]);
ctx.set::<Blog>();
ctx.ensure_created().await?;  // 建表后自动插入种子数据
```

## 新装：history-free bootstrap（bootstrap → seed）

空库安装不需要经过迁移历史表，也不需要调用方过滤生成的 history `INSERT` 或删除历史表：

```rust
// 一次调用：建全量 schema（表 + 索引 + 外键）+ 插入 has_data 声明的种子行
ctx.bootstrap().await?;
```

- `DbContext::bootstrap` 是 `bootstrap → seed` 的一条路径；引擎级入口为
  `MigrationEngine::bootstrap`（只建 schema）与 `bootstrap_with_seed`（建 schema + 种子）。
- 不创建、不写入 `__ef_migrations_history`；安装/探测状态仍由业务自己记录（如安装标记表）。
- 目标库必须为空：bootstrap 生成严格 `CREATE TABLE`（不是 `IF NOT EXISTS`），非空库会显式报错，
  避免把旧 schema 与当前模型静默混合。
- 种子行按 `bootstrap_with_seed` 的 `seeds` 顺序插入，有外键依赖时先插入主表行。
- 与 `ensure_created` 的区别：bootstrap 包含独立索引与外键且不写历史表；`ensure_created` 只建表
  （幂等 DDL，无独立索引/外键），适合测试与快速原型。注意 `DbContext::ensure_created` 会额外插入
  `has_data` 种子，引擎层 `MigrationEngine::ensure_created` 才是纯 DDL；`DbContext::bootstrap` 同样会插种子。

## CLI 工具

```bash
# 1. 应用内导出当前模型快照（见“自动升级流程”）
cargo run --bin snapshot

# 2. 添加迁移（--snapshot 是当前快照，Migrations/model_snapshot.json 是上次基线）
rust-ef migration add AddUserTable \
  --snapshot current_snapshot.json --dir Migrations --dialect sqlite

# 3. 应用迁移（或启动时 ctx.migrate("Migrations").await?）
rust-ef migration apply --connection "sqlite:app.db" --dir Migrations

# 回滚
rust-ef migration revert --connection "sqlite:app.db" --dir Migrations --target PreviousMigration

# 生成脚本
rust-ef migration script --dir Migrations --from InitialCreate --to AddUserTable
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 迁移文件纳入版本控制 | 确保团队 schema 一致 |
| 生产环境先备份再 apply | 迁移不可逆，Down SQL 仅用于开发回滚 |
| 种子数据仅用于静态枚举 | 动态业务数据不应通过 `has_data` 插入 |

下一章：[DI 与拦截器](../10-di-interceptors/INDEX.md)
