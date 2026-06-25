# 性能优化技巧

## 查询优化

### 用 `any()` 替代 `count() > 0`

```rust
// ❌ 扫描全表或索引
let exists = ctx.set::<Blog>().query().count().await? > 0;

// ✅ 找到第一条即停止
let exists = ctx.set::<Blog>().query().any().await?;
```

### 分页前先过滤

```rust
// ✅ 先过滤，再排序分页
let page = ctx.set::<Post>()
    .filter(linq!(|p: Post| p.blog_id == target_id))
    .order_by_desc("created_at")
    .skip(offset)
    .take(limit)
    .to_list()
    .await?;
```

### 按需 Include

```rust
// ❌ 加载不需要的导航
let blogs = ctx.set::<Blog>().query().include_named("posts").to_list().await?;

// ✅ 只在需要时 Include
let blogs = ctx.set::<Blog>().query().to_list().await?;
// 另一处代码需要 posts 时再查
```

## 批量操作优化

| 场景 | 低效做法 | 高效做法 |
|------|----------|----------|
| 批量更新 | `to_list()` + 逐条 `update()` | `execute_update()` |
| 批量删除 | `to_list()` + `remove_range()` | `execute_delete()` |
| 全表更新 | 逐条加载修改 | `load_all()` + `detect_changes()` |

## 连接管理

| 实践 | 说明 |
|------|------|
| 使用连接池 | Provider 内部通常已池化，避免频繁创建连接 |
| 一个请求一个 DbContext | DbContext 持有连接引用，不应跨请求复用 |
| 避免长事务 | 事务内不执行 HTTP 请求、文件 IO |

## 设计要点

| 实践 | 说明 |
|------|------|
| 聚合函数优先 | `sum()` / `avg()` 比 `to_list()` 再内存计算快得多 |
| 小结果集用 `to_list()` | 大结果集考虑流式处理或分页 |
| 索引配合查询 | 高频过滤列在实体上加 `#[index]`，辅助 MigrationEngine 生成 DDL |

下一节：[代码审查清单](code-review-checklist.md)
