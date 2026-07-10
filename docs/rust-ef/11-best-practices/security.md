# 安全最佳实践

rust-ef 在设计层面将 SQL 注入防护作为核心目标。本文档总结框架的安全机制、使用时的安全注意事项，以及生产部署的加固建议。

---

## 1. SQL 注入防护

### 参数化查询（核心防线）

rust-ef 的所有运行时值均通过 **参数化查询**（prepared statements）绑定到 SQL，绝不通过字符串拼接注入用户输入：

```rust
// linq! 宏生成的 WHERE 条件 — 值被包装为 DbValue，以占位符绑定
let blogs = linq!(ctx.set::<Blog>(), |b: Blog| b.url == user_input)
    .to_list()
    .await?;
// 生成: SELECT ... WHERE "url" = $1  ← user_input 作为参数 $1 传入，非拼接
```

**底层机制**：

| 层级 | 行为 |
|------|------|
| `linq!` 宏 | 将字面量值编译为 `DbValue::from(value)`，列引用编译为编译期 `&'static str` 常量 |
| `QueryBuilder` | WHERE 条件编译为 `BoolExpr`，值收集到 `Vec<DbValue>`，SQL 中仅出现占位符（`?` / `$N`） |
| Provider | `DbValue` 通过驱动级 `ToSql` / `bind` 绑定到 prepared statement |

三个 Provider（SQLite / PostgreSQL / MySQL）均使用驱动原生参数绑定：
- SQLite: `rusqlite` 的 `execute(sql, &params)`
- PostgreSQL: `tokio-postgres` 的 `client.execute(sql, &params)`
- MySQL: `sqlx` 的 `.bind(param)`

### 标识符来源

SQL 中的表名、列名均来自 **编译期实体元数据**（`#[derive(EntityType)]` 生成的 `TABLE` 常量与 `COLUMN_*` 常量），不接受运行时用户输入。`format!` 宏仅用于占位符生成和标识符引用，不参与值拼接。

### `BoolExpr::Raw` 的安全边界

`BoolExpr::Raw` 变体使用 `pub(crate) struct RawSql` 包装原始 SQL，外部代码无法命名该类型、无法构造 `Raw` 变体。`BoolExpr::raw()` 构造器同样为 `pub(crate)`。框架内部仅使用硬编码默认值 `"1=1"`（无条件查询的占位）。`linq!` 宏与 `QueryBuilder` 公开 API 均无法注入原始 SQL。

---

## 2. 迁移脚本安全

### 设计信任模型

`execute_migration_command(sql)` 直接执行原始 SQL 字符串。这是 **设计行为** — DDL 语句（CREATE TABLE / ALTER TABLE）无法参数化。

**信任边界**：迁移脚本由 **开发者编写**，存储在 `Migrations/<id>/up.sql` 中，属于受信代码，不接受运行时用户输入。

### 安全建议

- 迁移脚本中若需插入数据，优先使用 `DbContext::save_changes()` 而非手写 INSERT
- 迁移脚本文件不应包含来自用户输入的动态内容
- `{migration_id}` 占位符替换值来自文件名，属开发者控制

---

## 3. 连接字符串安全

### 存储与传递

连接字符串在 `DbContextOptionsBuilder` 配置阶段设定，属 **部署配置**，不应来自 HTTP 请求等运行时用户输入：

```rust
// ✅ 正确：从环境变量读取
let cs = std::env::var("DATABASE_URL")?;
builder.connection_string(&cs);

// ✅ 正确：硬编码或配置文件
builder.use_sqlite("app.db");

// ❌ 危险：从用户请求中获取
builder.connection_string(&user_supplied_url);  // 绝不要这样做
```

### 密码保护

- PostgreSQL 连接串中的密码由 `tokio-postgres` 标准库解析，支持 `password=...` 格式
- 推荐使用环境变量或秘密管理服务（如 Vault）注入密码，不要硬编码在源码中
- 连接池（deadpool）配置中的密码同样应从安全来源获取

### TLS / 传输加密

自 v1.6 起，PostgreSQL 与 MySQL Provider 均采用 **secure-by-default** 策略——`new()` 默认强制 TLS，`new_insecure()` 为本地开发逃生舱。

```rust
use rust_ef_postgres::{PostgresProvider, PgTlsMode};

// 方式 1: secure-by-default（v1.6+，生产推荐）
// 使用平台默认根证书库构建 TlsConnector
let provider = PostgresProvider::new(&url, 5)?;

// 方式 2: 明文连接（仅本地开发）
let provider = PostgresProvider::new_insecure(&url, 5)?;

// 方式 3: 自定义 CA 证书
let connector = native_tls::TlsConnector::builder()
    .add_root_certificate(/* 加载 CA 证书 */)
    .build()?;
let provider = PostgresProvider::new_with_tls(
    &url, 5, PgTlsMode::Require(connector)
)?;
```

```rust
use rust_ef_mysql::{MySqlProvider, MySqlTlsMode};

// secure-by-default（v1.6+）
let provider = MySqlProvider::new(&url).await?;          // 强制 Required
let provider = MySqlProvider::new_lazy(&url)?;           // 强制 Required（延迟连接）

// 明文连接（仅本地开发）
let provider = MySqlProvider::new_insecure(&url).await?;
let provider = MySqlProvider::new_lazy_insecure(&url)?;
```

`PgTlsMode::Require` 使用平台原生 TLS 实现（Windows SChannel / Linux OpenSSL / macOS Secure Transport）。TLS 类型在 `deadpool_postgres::Manager` 内部通过 `Box<dyn Connect>` 擦除，因此 `Pool` 与 `PostgresConnection` 保持非泛型 API — TLS 是构造期决策，非类型参数。

> **SQLite**：进程内数据库，无需 TLS。生产部署若跨不可信网络，仍建议结合 SSH 隧道或 VPN。
>
> **DI 扩展**：`use_postgres` / `use_mysql` 委托 `new()` / `new_lazy()`，自动 secure-by-default。逃生舱为 `use_postgres_with_tls(cs, pool, PgTlsMode::Disable)` / `use_mysql_with_tls(cs, MySqlTlsMode::Disabled)`。

---

## 4. 敏感字段映射

### 密码字段

rust-ef 不内置密码哈希 — 这是应用层的职责。推荐模式：

```rust
#[derive(Debug, Clone, EntityType)]
#[table("users")]
struct User {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    username: String,
    // 密码哈希存储为 String，应用层负责哈希
    #[required]
    #[max_length(255)]
    password_hash: String,
}

// 注册时：应用层哈希后再交给 DbContext
let user = User {
    id: 0,
    username: form.username,
    password_hash: argon2::hash_password(&form.password, &salt)?,
};
ctx.add::<User>(user);
ctx.save_changes().await?;
```

### 敏感数据查询

查询包含敏感字段时，注意不要将完整实体暴露在日志或 API 响应中：

```rust
// ✅ 使用 select 投影，只取需要的字段
let user_info = linq!(ctx.set::<User>(); select (b.id, b.username))
    .to_list()
    .await?;
// 不查询 password_hash，避免意外泄露
```

---

## 5. 全局查询过滤器与多租户

全局查询过滤器（如软删除 `is_deleted = 0`、租户隔离 `tenant_id = ?`）是安全纵深的重要一环 — 它确保开发者不会意外忘记 WHERE 条件：

```rust
// 应用启动时注册全局过滤器
builder.has_query_filter(linq!(filter |b: Blog| b.tenant_id == current_tenant_id()));
```

> **注意**：`query_ignore_filters()` 会绕过全局过滤器。在多租户场景中，仅在确认安全的情况下使用（如管理员后台的全局查询）。

---

## 6. 生产部署加固清单

| 项 | 建议 | 优先级 |
|----|------|:------:|
| 连接字符串来源 | 环境变量 / 秘密管理服务，不硬编码 | 高 |
| 数据库传输加密 | TLS / SSH 隧道 / VPN | 高 |
| 数据库账号权限 | 最小权限原则，应用账号无 DDL 权限 | 高 |
| 迁移脚本审查 | Code review 时检查迁移 SQL | 中 |
| 日志脱敏 | 不记录完整 SQL + 参数（可能含敏感值） | 中 |
| 连接池配置 | 设置合理的 max_connections / idle_timeout | 中 |
| 定期依赖更新 | `cargo audit` 检查依赖漏洞 | 中 |

---

## 审计结论

**v1.0 安全审计状态: ✅ 通过**

- SQL 注入：所有运行时值参数化绑定，无字符串拼接注入面
- 标识符来源：编译期实体元数据，不接受运行时输入
- 连接字符串：部署配置层，非运行时用户输入
- 迁移脚本：开发者受信代码，设计行为
- 已知限制：无（v1.6 起 `new()` 默认强制 TLS，`new_insecure()` 为本地开发逃生舱）

---

下一节：[代码审查清单](code-review-checklist.md)
