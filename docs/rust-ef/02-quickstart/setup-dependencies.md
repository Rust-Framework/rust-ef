# 环境准备与依赖

## 最小依赖集

```toml
[dependencies]
rust-ef = "2.1"
rust-ef-sqlite = "2.1"
rust-dix = "0.10"
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

## 可选 feature

| feature | 作用 | 默认 |
|---|---|---|
| `serde` / `chrono` / `uuid` / `decimal` | 为实体字段启用对应类型的映射 | 关 |
| `tracing` | 查询/保存路径的 tracing 日志 | 关 |
| `webx` | **rust-webx 集成**：提供 `impl From<EFError> for webx_core::error::Error`，让 `?` 在 Handler（`ActionResult<T>`）里直接传播持久层错误 | 关 |

`webx` feature 会引入可选依赖 `rust-webx-core`（发布范围 `^1.2`，兼容 rust-webx 1.3），其余能力与依赖完全不变。映射语义与 rust-webx 1.3 的 `Error` / `Error::public_detail` 对齐：

| `EFError` | HTTP | 客户端可见 detail |
|---|---|---|
| `NotFound` | 404 | 调用方业务文案 |
| `ModelValidation`、`Query`、`TypeConversion` | 400 | 校验类原样；查询/类型转换类固定文案（不泄露 SQL） |
| `ConcurrencyConflict`、`UniqueViolation` | 409 | 业务文案；唯一冲突固定文案（不泄露 constraint/table） |
| `Connection`、`Transaction` | 503 | 固定文案（不泄露连接串） |
| `Migration`、`Configuration`、`Provider`、`ChangeTracking`、`Other` | 500 | 无（`public_detail()` 为 `None`，原文只进日志） |

```toml
# 直接依赖
rust-ef = { version = "2.4.0", features = ["webx"] }

# 按 Travel 习惯重命名包名时的等价写法
ef = { package = "rust-ef", version = "2.4.0", features = ["webx"] }
```

```rust
use rust_ef::error::EFError;
use webx_core::error::ActionResult;

async fn handler() -> ActionResult<u32> {
    let value = load().await?; // EFError -> webx::Error（From impl）
    Ok(value)
}
```

显式映射用 `rust_ef::webx::map_ef_error(...)`（重命名依赖下为 `ef::webx::map_ef_error(...)`），返回 `webx_core::error::Error`，可直接 `?` 进 `ActionResult`；映射语义见上表。

## 开发工具

```bash
# CLI 工具（2.x 未发布到 crates.io，从仓库构建）
cargo install --path crates/cli

# 常用命令
rust-ef migration add InitialCreate --output ./Migrations
rust-ef migration list --connection "sqlite:app.db"
```

下一节：[定义第一个实体](first-entity.md)
