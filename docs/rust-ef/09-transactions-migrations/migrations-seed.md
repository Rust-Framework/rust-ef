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

## 种子数据

```rust
ctx.model().entity::<Blog>().has_data(&[
    Blog { blog_id: 1, url: "https://seed.example".into(), rating: 5 },
]);
ctx.set::<Blog>();
ctx.ensure_created().await?;  // 建表后自动插入种子数据
```

## CLI 工具

```bash
# 添加迁移
rust-ef migration add AddUserTable --output ./Migrations

# 应用迁移
rust-ef migration apply --connection "sqlite:app.db"

# 回滚
rust-ef migration revert --connection "sqlite:app.db" --target PreviousMigration

# 生成脚本
rust-ef migration script --from InitialCreate --to AddUserTable
```

## 设计要点

| 实践 | 说明 |
|------|------|
| 迁移文件纳入版本控制 | 确保团队 schema 一致 |
| 生产环境先备份再 apply | 迁移不可逆，Down SQL 仅用于开发回滚 |
| 种子数据仅用于静态枚举 | 动态业务数据不应通过 `has_data` 插入 |

下一章：[DI 与拦截器](../10-di-interceptors/INDEX.md)
