# EnsureCreated 与 EnsureDeleted

## EnsureCreated

根据已注册的实体类型自动创建表：

```rust
let mut ctx = DbContext::from_options(&options)?;
ctx.set::<Blog>();
ctx.set::<Post>();

// 自动创建 blogs 表和 posts 表
ctx.ensure_created().await?;
```

同时会执行 `has_data` 注册的种子数据插入。

## EnsureDeleted

删除所有已注册实体对应的表：

```rust
ctx.ensure_deleted().await?;
```

## 适用场景

| 场景 | 建议 |
|------|------|
| 单元测试 | 每个测试用 `ensure_created` + `ensure_deleted`，保持隔离 |
| 快速原型 | 无需手写 DDL，启动即自动建表 |
| 生产环境 | **不推荐**直接用于生产，应使用 MigrationEngine 管理 schema 变更 |

## 设计要点

| 实践 | 说明 |
|------|------|
| 调用前必须先 `set::<T>()` | 否则 `ensure_created` 不知道要建哪些表 |
| 内存测试优先 | SQLite `:memory:` + `ensure_created` 是最快的集成测试方案 |

下一节：[迁移引擎与种子数据](migrations-seed.md)
