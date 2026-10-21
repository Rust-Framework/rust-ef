# 第十章 DI 与拦截器

## 本章小节

| 小节 | 内容 |
|------|------|
| [rust-dix 集成与注册模式](di-registration.md) | `add_dbcontext`、provider 解析与生命周期 |
| [多数据库 Keyed 注册](keyed-databases.md) | `add_dbcontext_keyed` 与读写分离 |
| [SaveChanges 拦截器](save-changes-interceptors.md) | 审计、软删除与验证钩子 |

## 实战教程

| 教程 | 内容 |
|------|------|
| [读写分离实战：主库写入、副本查询](../12-tutorials/read-write-splitting.md) | keyed 双 context、三库配置、read-your-writes、只读账号、延迟与排错 |

## 下一步

从 [rust-dix 集成与注册模式](di-registration.md) 开始。
