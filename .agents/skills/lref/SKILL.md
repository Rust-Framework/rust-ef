---
name: lref
description: |
  Rust Entity Framework (REF) ORM 框架开发指南。涵盖实体定义、linq! 查询、DbContext�?  DI 集成、Web 应用集成、软删除、变更追踪。当用户编写或修�?REF 相关代码时使用�?---

# REF 框架开发指�?
Rust Entity Framework �?接口驱动�?EFCore 风格 ORM。本指南�?*渐进式披�?*组织�?先掌握高频基础，再深入进阶模式，最后了解避坑要点�?
> **rust-webapp 就相当于 ASP.NET Core**：`add_dbcontext` �?`AddDbContext<T>`�?> Handler 注入 �?构造函数注入，`IRequestHandler` �?Controller Action�?> 详见[第二层：Web 应用集成](references/webapp-integration.md)�?
---

## 第一层：快速入�?
> 90% 的开发场景只需掌握本层内容。详�?[references/quickstart.md](references/quickstart.md)

| 主题 | 要点 |
|------|------|
| 实体定义 | `#[derive(EntityType)]` + `#[table]` + `#[primary_key]` + `#[auto_increment]` |
| 查询 | `let expr = linq!(\|b: Blog\| b.rating > 3);` �?`set.filter(expr).to_list()` |
| 增删�?| `add()` �?`save_changes()`；`detect_changes()` / `update()` �?`save_changes()` |
| DI 注册 | `add_dbcontext(|o| o.use_sqlite(...))` |

**记住两条核心原则�?*

1. **分步绑定，不链式**：`let query = ctx.set::<Blog>().query(); let blog = query.find(1).await?;`
2. **linq! 表达式绑�?*：`let expr = linq!(|b: Blog| b.slug == "hello"); set.filter(expr).first_or_default()`

---

## 第二层：Web 应用集成

> rust-webapp 生产环境核心参考。详�?[references/webapp-integration.md](references/webapp-integration.md)

| 主题 | 对标 ASP.NET Core | 要点 |
|------|:---:|------|
| 上下文注�?| `AddDbContext<T>` | `add_dbcontext` 注册�?**Scoped**，每个请求独立实�?|
| Handler 注入 | 构造函数注�?| `Arc<dyn IDbContext>` �?DI 容器自动解析 |
| 读取操作 | `[HttpGet]` | 分页 + 导航 + 排序一�?linq! 完成 |
| 创建操作 | `[HttpPost]` | 唯一性校�?�?插入 �?save_changes �?按主键回查导�?|
| 更新操作 | `[HttpPut]` | 加载 �?权限校验 �?应用变更 �?detect_changes �?保存 |
| 删除操作 | `[HttpDelete]` | 软删除：加载 �?标记 �?detect_changes �?保存 |
| 错误处理 | `IActionResult` | 5 种错误类型映�?|
| 服务�?| Service 抽象 | 复杂业务逻辑抽取 `I...Service` |

**核心设计：Scoped 生命周期，无需�?*

`add_dbcontext` 注册�?Scoped，每个请求通过 DI Scope 获得独立�?`DbContext` 实例�?天然隔离，无需 `Arc<Mutex<>>`。这�?EFCore 的设计范式�?
> **`Arc<Mutex<DbContext>>` 是反模式**：会导致跨请求跟踪污染、并发竞争和性能退化�?> 详见 [references/pitfalls.md#p1](references/pitfalls.md#p1-arcmutexdbcontext-反模�?�?
---

## 第三层：深入理解

> 进阶功能，按需查阅。详�?[references/advanced.md](references/advanced.md)

| 主题 | 要点 |
|------|------|
| linq! 三种形式 | Form A 过滤闭包、Form B 多子句、Form C ModelBuilder 配置 |
| 批量操作 | `execute_update` / `execute_delete` |
| 可复用表达式 | `let expr = linq!(|b: Blog| ...);` 复用于多终端 |
| SaveChanges 拦截�?| `ISaveChangesInterceptor` 审计日志 |
| 多数据库 | `add_dbcontext_keyed` + `#[context("key")]` |

---

## 第四层：避坑指南

> 生产环境已验证的反模式和已知限制。详�?[references/pitfalls.md](references/pitfalls.md)

| 陷阱 | 说明 |
|------|------|
| `Arc<Mutex<DbContext>>` | 反模式：跟踪污染、虚假并发竞�?|
| `save_changes()` 后回�?ID | 自增 ID 已自动填充到实体 |
| 插入后按非唯一字段回查 | 并发场景下不保证取到自己的记�?|
| 字符串列�?API | 无编译期检查，已移�?|
| 每条查询重复�?`is_deleted` | 用全局查询过滤器注册一�?|
| `detect_changes()` vs `update()` | 前者仅标记实际变更字段 |

---

## 模板文件

| 模板 | 用�?|
|------|------|
| [templates/entity-definition.rs](templates/entity-definition.rs) | 完整实体定义示例 |
| [templates/web-handler-crud.rs](templates/web-handler-crud.rs) | Web Handler CRUD 完整模板 |
| [templates/soft-delete.rs](templates/soft-delete.rs) | 软删除完整模�?|
| [templates/di-setup.rs](templates/di-setup.rs) | DI 注册配置模板 |

---

## 可运行示�?
| 示例 | 路径 |
|------|------|
| Blog 完整示例 | `examples/blog/src/main.rs` |
| 软删除示�?| `examples/soft_delete/src/main.rs` |

## 框架文档

| 文档 | 路径 |
|------|------|
| 完整文档索引 | `docs/rust-ef/INDEX.md` |
| 快速入�?| `docs/rust-ef/02-quickstart/INDEX.md` |
| 查询模式 | `docs/rust-ef/05-query-patterns/INDEX.md` |
| 最佳实�?| `docs/rust-ef/11-best-practices/INDEX.md` |