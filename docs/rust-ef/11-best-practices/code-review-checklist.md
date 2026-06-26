# 代码审查清单

## 实体层

- [ ] 所有实体是否派生了 `Clone` 和 `Debug`？
- [ ] 主键是否标注了 `#[primary_key]`？
- [ ] 字符串必填列是否加了 `#[required]`？
- [ ] 导航属性是否加了 `#[navigation]`？
- [ ] 外键是否标注了 `#[foreign_key(Entity)]`？

## 查询层

- [ ] `linq!` 宏是否包含类型标注（`|e: Entity|`）？
- [ ] 复杂查询是否拆分为独立的 `let` 绑定？
- [ ] 是否优先用 `any()` 替代 `count() > 0`？
- [ ] 分页查询是否先过滤再 `skip`/`take`？
- [ ] 导航属性使用前是否用 `linq!(...; include b.x)` 预加载？
- [ ] 形式 B 的 source 是否含 turbofish `::<Type>`（不能用裸变量）？
- [ ] 是否已移除所有字符串 API（`include_named`/`order_by("col")`/`sum("col")`/`find_by_id` 等）？

## 变更层

- [ ] 修改实体后是否调用了 `update()` 或 `detect_changes()`？
- [ ] `save_changes()` 是否覆盖了一个业务操作的全部变更？
- [ ] 批量操作是否使用了 `execute_update()` / `execute_delete()`？
- [ ] 删除前是否确认了过滤条件，避免误删全表？

## 事务与生产

- [ ] `ensure_created()` 是否在 `set::<T>()` 之后调用？
- [ ] 生产环境 schema 变更是否通过 MigrationEngine 管理？
- [ ] 拦截器是否按正确顺序注册（审计 → 验证 → 软删除）？
- [ ] 原始 SQL 是否集中管理并经过参数化审查？

## 小结

本清单覆盖了从实体定义到生产部署的核心检查点。建议在 PR 提交前逐项核对，可显著减少线上问题。

---

> 本书至此结束。更多问题请参考 [Rust Entity Framework 开发者手册 · 目录](../INDEX.md)。
