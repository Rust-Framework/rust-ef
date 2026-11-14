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

### 连接串：URL 与 ADO.NET（2.5.0）

`ConnectionString` 支持两种部署环境常见形态（EF-CONN-01）：

```rust
use rust_ef::connection_string::{ConnectionProtocol, ConnectionString};

// URL：userinfo/查询值会百分号解码，build() 再编码，密码可含 @ & # 空格
let url = ConnectionString::parse("postgres://travel:p%40ss@db:5432/travel?sslmode=require")?;

// ADO.NET：source/server/host + uid/pwd + database/initial catalog，容忍尾随分号
let ado = ConnectionString::parse_ado(
    "Source=db.example:5544;Uid=travel;Pwd=p&ss=word#1;Database=travel;",
    ConnectionProtocol::Postgres,
)?;

// 自动识别：含 "://" 走 URL，否则按 ADO.NET 解析
let auto = ConnectionString::parse_auto(&ado.build()?, ConnectionProtocol::Postgres)?;
```

- 键大小写不敏感；`data source`/`address`、`user id`/`username`、`pwd`/`password`、
  `database`/`initial catalog` 等别名均可用；host 内的 `:port` 优先于单独的 `port=`。
- `sslmode` / `ssl-mode` / `ssl mode` 会规范化为 URL 的 `sslmode`（PG）或 `ssl-mode`（MySQL），
  值走同一白名单校验；`false`/`true` 映射为 disable/require 系列。
- 服务端 TLS 构造语义统一为 **secure by default**：`use_postgres`/`PostgresProvider::new`
  在 URL 没有 `sslmode` 时要求 TLS，`sslmode=disable`（或 `allow`）显式允许明文；
  `use_mysql`/`MySqlProvider::new` 在 URL 没有 `ssl-mode` 时要求 TLS，显式 `ssl-mode=disabled`
  即明文。需要强制覆盖时用 `insecure()` / `use_*_with_tls`。

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
| `Database` | 500 | 无（结构化驱动错误含 SQLSTATE/SQL，只进日志） |
| `Migration`、`Configuration`、`Provider`、`ChangeTracking`、`Other` | 500 | 无（`public_detail()` 为 `None`，原文只进日志） |

```toml
# 直接依赖
rust-ef = { version = "2.5.0", features = ["webx"] }

# 按 Travel 习惯重命名包名时的等价写法
ef = { package = "rust-ef", version = "2.5.0", features = ["webx"] }
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
