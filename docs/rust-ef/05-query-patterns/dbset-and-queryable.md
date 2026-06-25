# DbSet 与 IQueryable 入门

## DbSet 是什么

`DbSet<T>` 是类型化的实体集合，提供两个核心能力：
- **变更操作**（`IDbSet<T>`）：`add`、`update`、`remove_at`
- **查询入口**（`IQueryable<T>`）：`query()` 返回 `QueryBuilder<T>`

## 获取 DbSet

```rust
let set = ctx.set::<Blog>();
```

`set::<T>()` 是 lazy 的：首次调用创建，后续调用返回同一实例。

## QueryBuilder 生命周期

```rust
let set = ctx.set::<Blog>();

// query() 创建新的 QueryBuilder，不修改 DbSet 状态
let q1 = set.query();
let q2 = set.query();

// 两者独立，互不影响
let all = q1.to_list().await?;
let first = q2.take(1).to_list().await?;
```

## 推荐写法

```rust
let set = ctx.set::<Blog>();
let expr = linq!(|b: Blog| b.rating > 0.5);
return set.filter(expr).to_list().await?;
```

这种写法比全部链式调用更清晰：
- `set` —— 数据来源明确
- `expr` —— 过滤逻辑独立命名
- 终端方法 —— 最后一步才执行查询

## 设计要点

| 实践 | 说明 |
|------|------|
| `set` 复用 | 同一 `DbSet` 可发起多次独立查询 |
| 查询不修改跟踪状态 | `query()` 只读取数据库，不影响 `DbSet.entries` |

下一节：[linq! 宏：推荐写法与可读性](linq-macro.md)
