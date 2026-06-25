# 环境准备与依赖

## 最小依赖集

```toml
[dependencies]
rust-ef = "0.3"
rust-ef-sqlite = "0.3"
rust-dicore = "0.2"
tokio = { version = "1", features = ["full"] }
```

## 多 Provider 切换

| 数据库 | 依赖 | 注册方式 |
|--------|------|----------|
| SQLite | `rust-ef-sqlite` | `.use_sqlite("app.db")` |
| PostgreSQL | `rust-ef-postgres` | `.use_postgres("host=localhost/db")` |
| MySQL | `rust-ef-mysql` | `.use_mysql("mysql://root:pass@localhost/db")` |

## 快速验证

```rust
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let ctx = DbContext::from_options(&builder.build())?;
    println!("DbContext created successfully!");
    Ok(())
}
```

## 开发工具

```bash
# CLI 工具（开发时依赖）
cargo install rust-ef-cli

# 常用命令
rust-ef migration add InitialCreate --output ./Migrations
rust-ef migration list --connection "sqlite:app.db"
```

下一节：[定义第一个实体](first-entity.md)
