# 第十二章 实战教程

前两篇是端到端教程，把事务与读写分离从 API 说明落到可直接改造的业务代码：

| 教程 | 内容 |
|------|------|
| [事务实战：转账、批量 DML 与业务回滚](transactions.md) | `transaction` / `TxOutcome`、批量 DML、查询加入事务、隔离级别、手动句柄、测试与避坑 |
| [读写分离实战：主库写入、副本查询](read-write-splitting.md) | keyed 双 `DbContext`、SQLite/MySQL/PostgreSQL 配置、read-your-writes、副本延迟、只读账号、排错 |

两篇都假设你已读过：

- [手动事务：use_transaction 与 transaction](../09-transactions-migrations/manual-transactions.md)
- [多数据库 Keyed 注册](../10-di-interceptors/keyed-databases.md)

> 示例用 `f64` 表示金额只为简化代码；真实资金系统请使用 `rust_decimal::Decimal` 或整数最小货币单位。
