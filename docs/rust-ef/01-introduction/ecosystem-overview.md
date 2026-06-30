# 生态与 Crate 全景

`rust-ef` 采用 Workspace 多 Crate 架构，核心与 Provider 分离，按需引入依赖。

## Crate 分层

```
User Application
    ??? rust-dicore (DI 容器，解析 Arc<DbContext>)
    ??? rust-ef (ORM 核心)
          DbContext (type-map 存储，无实体特定字段，具体上下文类型)
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
rust-ef = { version = "1.3", features = ["chrono", "uuid", "decimal"] }
rust-ef-sqlite = "1.3"
rust-dicore = "0.5.1"
tokio = { version = "1", features = ["full"] }

[dev-dependencies]
rust-ef-cli = "1.3"
```

## 可选 Feature（v0.5 已实现）

| Feature | 说明 | 依赖 |
|---------|------|------|
| `chrono` | `DateTime<Utc>` / `NaiveDateTime` / `NaiveDate` 映射 | `chrono` crate |
| `uuid` | `uuid::Uuid` 类型支持 | `uuid` crate（含 `v4`） |
| `decimal` | `rust_decimal::Decimal` 高精度小数 | `rust_decimal` crate |
| `serde` | `DbValue` 等类型的 Serde 序列化 | `serde` crate |

启用方式：在 `rust-ef` 的 features 中声明对应 feature，宏与核心库会自动启用对应的 `From` impl 与 `from_row` 解析分支。详见 [类型映射参考表](../03-entity-design/type-mapping.md)。

## 小结

`rust-ef` 的核心设计哲学是**解耦**：ORM 逻辑与数据库方言解耦，查询构造与执行解耦，实体定义与持久化解耦。这种分层使你可以只换 Provider 而无需修改业务代码。

下一章：[快速上手](../02-quickstart/INDEX.md)
