# 生态与 Crate 全景

`rust-ef` 采用 Workspace 多 Crate 架构，核心与 Provider 分离，按需引入依赖。

## Crate 分层

```
User Application
    ??? rust-dicore (DI 容器，解析 Arc<dyn IDbContext>)
    ??? rust-ef (ORM 核心)
          DbContext (type-map 存储，无实体特定字段)
          ??? IDbContext      ??object-safe session trait
          ??? IDbSet<T>       ??实体集合（变更操作）
          ??? IQueryable<T>   ??查询入口
          ??? ISaveChangesInterceptor ??提交前后钩子
          ??? IDatabaseProvider ??后端抽象
                ??? rust-ef-sqlite     (use_sqlite)
                ??? rust-ef-postgres   (use_postgres)
                ??? rust-ef-mysql      (use_mysql)
```

## 各 Crate 职责

| Crate | 职责 | 是否必须 |
|-------|------|:--------:|
| `rust-ef` | 核心：DbContext、DbSet、QueryBuilder、ChangeTracker、MigrationEngine | ✅ |
| `rust-ef-macros` | `#[derive(EntityType)]` 与 `linq!` 宏 | ✅（由 core  re-export） |
| `rust-ef-sqlite` | SQLite Provider：`SqliteProvider`、`use_sqlite` | 按需 |
| `rust-ef-postgres` | PostgreSQL Provider：`use_postgres` | 按需 |
| `rust-ef-mysql` | MySQL Provider：`use_mysql` | 按需 |
| `rust-ef-cli` | 命令行工具：migration / scaffold | 开发时 |

## Cargo.toml 示例

```toml
[dependencies]
rust-ef = "0.3"
rust-ef-sqlite = "0.3"
rust-dicore = "0.2"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
rust-ef-cli = "0.3"
```

## Feature 规划（未来版本）

| Feature | 说明 |
|---------|------|
| `chrono` | `DateTime` / `NaiveDate` 映射 |
| `uuid` | UUID 类型支持 |
| `decimal` | `rust_decimal` 集成 |

## 小结

`rust-ef` 的核心设计哲学是**解耦**：ORM 逻辑与数据库方言解耦，查询构造与执行解耦，实体定义与持久化解耦。这种分层使你可以只换 Provider 而无需修改业务代码。

下一章：[快速上手](../02-quickstart/INDEX.md)
