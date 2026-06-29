# 适用场景与边界

## 推荐场景 ✅

| 场景 | 原因 |
|------|------|
| **Web 后端服务** | 与 `rust-webapp` 天然集成，`Arc<DbContext>` 可直接注入 Handler |
| **多实体关联业务** | `Include` / `ThenInclude` 处理一对多、多对多无需手写 JOIN |
| **需要审计/软删除** | `ISaveChangesInterceptor` 在提交前后统一注入横切逻辑 |
| **从 EF Core 迁移** | `DbContext`、`DbSet<T>`、`SaveChanges` 概念一一对应 |
| **快速原型与内部工具** | SQLite 内存模式 + `ensure_created()` 无需外部数据库即可启动 |

## 不推荐场景 ❌

| 场景 | 原因 |
|------|------|
| **极致性能敏感** | ORM 抽象层有一定开销；大量数据 ETL 建议直接用 `sqlx` |
| **高度动态 schema** | 实体类型在编译期确定，运行时动态表名/列名支持有限 |
| **复杂子查询与 CTE** | v0.3 尚未支持子查询和关联过滤，需退回到原始 SQL |
| **无异步 Runtime** | 所有数据库操作均为 `async`，需 `tokio` 或兼容 Runtime |

## 版本与成熟度

当前版本 **v0.3.x**，处于 **Beta/RC 过渡阶段**：

- ✅ SQLite 集成测试完备（46+ 测试全绿）
- ✅ PostgreSQL / MySQL Provider 已实现，集成测试需环境变量激活
- ⚠️ 乐观并发控制元数据已就绪，但完整冲突检测待完善
- ⚠️ CLI 工具（migration / scaffold）基础能力已具备

建议：SQLite 原型与内部工具可放心使用；PostgreSQL / MySQL 生产环境请先跑通集成测试再上线。

## 小结

选择 `rust-ef` 的核心判断标准：**你是否愿意为工程化便利（关系映射、变更跟踪、DI 集成）接受 ORM 的抽象成本**。如果是，继续阅读；如果追求零开销或完全可控的 SQL，裸写 `sqlx` 可能是更好的选择。

下一节：[生态与 Crate 全景](ecosystem-overview.md)
