# 第九章 事务与迁移

## 本章小节

| 小节 | 内容 |
|------|------|
| [手动事务：use_transaction 与 transaction](manual-transactions.md) | 事务覆盖范围、`TxOutcome` 回滚并返回结果、savepoint |
| [EnsureCreated 与 EnsureDeleted](ensure-created-deleted.md) | 快速原型与测试环境数据库管理 |
| [迁移引擎与种子数据](migrations-seed.md) | `MigrationEngine`、`has_data` 与 Up/Down SQL |

## 实战教程

| 教程 | 内容 |
|------|------|
| [事务实战：转账、批量 DML 与业务回滚](../12-tutorials/transactions.md) | 端到端业务代码、`TxOutcome`、隔离级别、手动句柄、测试与避坑 |

## 下一步

从 [手动事务：use_transaction 与 transaction](manual-transactions.md) 开始。
