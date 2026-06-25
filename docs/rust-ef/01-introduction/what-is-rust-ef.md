# 什么是 rust-ef

`rust-ef`（Rust Entity Framework）是一个**接口导向、EF Core 风格**的 Rust ORM 框架。它提供类型安全的实体映射、LINQ 风格的查询构造、变更跟踪与 Unit-of-Work 持久化，并与 `rust-dicore` DI 容器深度集成。

## 核心设计目标

| 目标 | 说明 |
|------|------|
| **类型安全** | 查询在编译期构造，`linq!` 宏将 Rust 表达式树翻译为参数化 SQL |
| **接口导向** | `IDbContext` / `IDbSet<T>` / `IQueryable<T>` 分离关注点，支持 `Arc<dyn IDbContext>` DI 解析 |
| **EF Core 思维模型** | `DbContext` → `DbSet<T>` → `SaveChanges`，熟悉 .NET 生态的开发者可快速上手 |
| **渐进式复杂度** | 简单查询一行完成，复杂场景拆分为多步 `let` 绑定 |

## 一句话定位

> 如果你需要 `sqlx` 的性能与 Rust 类型安全，又希望拥有 EF Core 级别的工程化 ORM 体验——`rust-ef` 是为你设计的。

## 与生态其他方案对比

| 方案 | 类型安全 | 关系映射 | DI 集成 | 查询风格 |
|------|:--------:|:--------:|:-------:|----------|
| `sqlx` (裸写) | ✅ 编译期检查 | ❌ 手动 | ❌ 无 | 原始 SQL |
| `sea-orm` | ✅ | ✅ | 可选 | Builder |
| `diesel` | ✅ | ✅ | 可选 | DSL |
| **rust-ef** | ✅ | ✅ | ✅ 原生 | `linq!` + Builder |

## 关键术语速览

| 术语 | 对应 EF Core | 说明 |
|------|-------------|------|
| `DbContext` | `DbContext` | 会话与工作单元入口，管理所有 `DbSet` |
| `DbSet<T>` | `DbSet<T>` | 类型化实体集合，提供查询与变更入口 |
| `linq!` | LINQ `Where` | 编译期表达式树宏，生成参数化过滤条件 |
| `SaveChanges` | `SaveChanges()` | 将已跟踪的增删改一次性提交到数据库 |
| `Include` | `Include()` | Eager Loading，预加载关联导航属性 |

## 小结

`rust-ef` 不是试图替代 `sqlx`，而是在其之上提供更高层次的抽象。当你需要处理多表关联、变更跟踪、批量操作和模块化 DI 注入时，它能让代码更聚焦业务逻辑，而非 SQL 拼接。

下一节：[适用场景与边界](who-should-use.md)
