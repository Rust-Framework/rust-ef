# Summary

[前言](./FOREWORD.md)

---

# 入门与基础

- [第一章 认识 rust-ef](./01-introduction/INDEX.md)
  - [什么是 rust-ef](./01-introduction/what-is-rust-ef.md)
  - [适用场景与边界](./01-introduction/who-should-use.md)
  - [生态与 Crate 全景](./01-introduction/ecosystem-overview.md)

- [第二章 快速上手](./02-quickstart/INDEX.md)
  - [环境准备与依赖](./02-quickstart/setup-dependencies.md)
  - [定义第一个实体](./02-quickstart/first-entity.md)
  - [DbContext 与 DI 注册](./02-quickstart/dbcontext-and-di.md)
  - [第一个 CRUD 流程](./02-quickstart/first-crud.md)
  - [自动注册与实体发现](./02-quickstart/auto-registration.md)

---

# 实体与关系

- [第三章 实体设计](./03-entity-design/INDEX.md)
  - [EntityType 派生与属性配置](./03-entity-design/derive-attributes.md)
  - [主键、自增与必填约束](./03-entity-design/keys-constraints.md)
  - [索引、唯一性与并发标记](./03-entity-design/indexes-concurrency.md)
  - [类型映射参考](./03-entity-design/type-mapping.md)

- [第四章 关系与导航](./04-relationships/INDEX.md)
  - [一对多与 BelongsTo / HasMany](./04-relationships/one-to-many.md)
  - [多对多与 Join 实体](./04-relationships/many-to-many.md)
  - [Eager Loading：Include 与 ThenInclude](./04-relationships/eager-loading.md)
  - [Lazy Loading：按需加载（v1.1）](./04-relationships/lazy-loading.md)

---

# 查询最佳实践

- [第五章 查询模式](./05-query-patterns/INDEX.md)
  - [DbSet 与 IQueryable 入门](./05-query-patterns/dbset-and-queryable.md)
  - [linq! 宏：统一 DSL 入口](./05-query-patterns/linq-macro.md)
  - [过滤、排序与分页](./05-query-patterns/filter-sort-page.md)
  - [计数与存在性检查](./05-query-patterns/count-any.md)

- [第六章 高级查询](./06-advanced-query/INDEX.md)
  - [聚合函数：SUM / AVG / MIN / MAX](./06-advanced-query/aggregation.md)
  - [GROUP BY 与 HAVING](./06-advanced-query/group-by-having.md)
  - [JOIN 查询](./06-advanced-query/join-queries.md)
  - [CTE 与 Window 函数（v1.1）](./06-advanced-query/cte-window-functions.md)
  - [全局查询过滤器](./06-advanced-query/global-query-filters.md)
  - [原始 SQL 与已知限制](./06-advanced-query/raw-sql-limitations.md)

---

# 变更与持久化

- [第七章 变更跟踪](./07-change-tracking/INDEX.md)
  - [Add / Attach / Update / Remove](./07-change-tracking/crud-states.md)
  - [SaveChanges 与事务边界](./07-change-tracking/save-changes.md)
  - [ChangeTracker 与 DetectChanges](./07-change-tracking/change-tracker.md)

- [第八章 批量操作](./08-bulk-operations/INDEX.md)
  - [批量更新 ExecuteUpdate](./08-bulk-operations/execute-update.md)
  - [批量删除 ExecuteDelete](./08-bulk-operations/execute-delete.md)
  - [RemoveRange 与 LoadAll](./08-bulk-operations/remove-range-load-all.md)

---

# 生产与工程化

- [第九章 事务与迁移](./09-transactions-migrations/INDEX.md)
  - [手动事务与 use_transaction](./09-transactions-migrations/manual-transactions.md)
  - [EnsureCreated 与 EnsureDeleted](./09-transactions-migrations/ensure-created-deleted.md)
  - [迁移引擎与种子数据](./09-transactions-migrations/migrations-seed.md)

- [第十章 DI 与拦截器](./10-di-interceptors/INDEX.md)
  - [rust-dicore 集成与注册模式](./10-di-interceptors/di-registration.md)
  - [多数据库 Keyed 注册](./10-di-interceptors/keyed-databases.md)
  - [SaveChanges 拦截器](./10-di-interceptors/save-changes-interceptors.md)

- [第十一章 最佳实践与避坑](./11-best-practices/INDEX.md)
  - [常见陷阱与排查](./11-best-practices/common-pitfalls.md)
  - [性能优化技巧](./11-best-practices/performance-tips.md)
  - [安全最佳实践](./11-best-practices/security.md)
  - [代码审查清单](./11-best-practices/code-review-checklist.md)

---

# 附录

- [多租户基础](./03-advanced/multi-tenancy-foundation.md)
