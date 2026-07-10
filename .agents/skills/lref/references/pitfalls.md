# 第四层：避坑指南

> 生产环境已验证的反模式和已知限制，请务必阅读。

## P1: `Arc<Mutex<DbContext>>` 反模式

`DbContext` 设计为**非线程安全**（对标 EFCore），共享实例会导致跨请求跟踪污染。
```rust
// ❌ 反模式：共享 DbContext 会导致 Thread A 的 save_changes()
//          提交 Thread B 挂起的变更
let ctx = Arc::new(Mutex::new(ctx));
```

**正确做法：Scoped 生命周期**

```rust
// ✅ 正确：add_dbcontext 注册为 Scoped，每个请求独立实例
let provider = ServiceCollection::new()
    .add_dbcontext(|o| o.use_sqlite("app.db"))
    .build()?;

// 推荐：owned 解析，直接 &mut self 访问，无需锁
let mut ctx: DbContext = provider.get_owned();

// 或：共享解析（同一 scope 内多次 get 返回同一实例）
// let scope = provider.create_scope();
// let ctx: Arc<DbContext> = scope.get();
```

从根 `ServiceProvider` 直接解析退化为 transient（每次新实例），安全但失去单位工作语义。
Handler 推荐使用 `get_owned()` 获取 owned `DbContext`，bare `T` 字段必须标记 `#[inject(owned)]`（`Arc<T>` 字段标记 `#[inject]`），未标记字段走 `Default::default()`。

## P2: save_changes() 后不要回查 ID

`save_changes()` 后自增 ID 已填充到实体上，无需额外查询。
```rust
// ❌ 错误：不必要的数据库往返
ctx.add::<Blog>(blog);
ctx.save_changes().await?;
let saved = linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q)
    .first_or_default().await?;
let id = saved.unwrap().id;

// ✅ 正确：直接用实体上的 id
ctx.add::<Blog>(blog);
ctx.save_changes().await?;
let id = blog.id; // 已自动填充！
```

## P3: 插入后不要按非唯一字段回查

```rust
// ❌ 错误：按 blog_id + user_id 回查并取 max(id) — 并发场景下不保证取到自己的记录
let created = linq!(ctx.set::<Comment>(), |c: Comment|
    c.blog_id == blog_id && c.user_id == user_id
).to_list().await?;
let last = created.into_iter().max_by_key(|c| c.id).unwrap();

// ✅ 正确：直接用实体上的 id
ctx.add::<Comment>(comment);
ctx.save_changes().await?;
let id = comment.id; // 已自动填充
```

## P4: 不要使用字符串列名 API

```rust
// ❌ 错误：无编译期检查，拼写错误运行时才发现
ctx.set::<Blog>().query()
    .filter_column("slug", "=", "hello")
    .order_by_column("publishd_at")  // 拼写错误！
    .to_list().await?;

// ✅ 正确：linq! 提供编译期类型检查
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == "hello";
    order_by b.published_at desc;
).to_list().await?;
```

## P5: 不要在每条查询中重复加 is_deleted 过滤

```rust
// ❌ 错误：重复且容易遗漏
linq!(ctx.set::<Blog>(), |b: Blog| b.slug == q && !b.is_deleted)
linq!(ctx.set::<User>(), |u: User| !u.is_deleted)
linq!(ctx.set::<Category>(), |c: Category| !c.is_deleted)

// ✅ 正确：启动时注册一次全局查询过滤器
ctx.model().entity::<Blog>()
    .has_query_filter(linq!(filter |b: Blog| !b.is_deleted));
ctx.model().entity::<User>()
    .has_query_filter(linq!(filter |u: User| !u.is_deleted));
// 所有查询自动排除已删除记录
```

## P6: 修改操作优先用 detect_changes() 而非 update()

```rust
// ❌ 不够精确：update() 标记整个实体为 Modified
ctx.update::<Blog>(blog);
ctx.save_changes().await?;

// ✅ 更好：detect_changes() 仅标记实际变更的字段
blog.is_deleted = true;
blog.updated_at = now;
ctx.detect_changes();
ctx.save_changes().await?;
```

## P7: 修改实体后忘记调用 update() 或 detect_changes()

```rust
// ❌ 错误：save_changes 不会提交这个修改
let mut blog = {
    let query = ctx.set::<Blog>().query();
    query.find(1).await?.unwrap()
};
blog.rating = 99;
ctx.save_changes().await?;  // 没有任何 UPDATE！

// ✅ 正确：显式标记修改
ctx.detect_changes();  // 或 update(blog)
ctx.save_changes().await?;
```

## P8: linq! 忘记类型标注

```rust
// ❌ 编译错误
let expr = linq!(|b| b.rating > 5);

// ✅ 正确
let expr = linq!(|b: Blog| b.rating > 5);
```

**原因**：`linq!` 是 `proc_macro`，在编译的解析后、类型检查前阶段执行。宏需要实体类型来将字段引用编译为列常量，但此时类型检查尚未运行，宏无法从上下文推断闭包参数的类型。

## P9: 在循环里逐条 save_changes()

```rust
// ❌ 性能极差，每次循环都开事务
for blog in blogs {
    ctx.add::<Blog>(blog);
    ctx.save_changes().await?;
}

// ✅ 正确：一次事务提交全部
for blog in blogs {
    ctx.add::<Blog>(blog);
}
ctx.save_changes().await?;
```

## P10: 导航属性为空因为没 include

```rust
// ❌ posts 为空（Lazy Loading 未开启时）
let blogs = ctx.set::<Blog>().query().to_list().await?;

// ✅ 方式一：用 linq! 的 include 子句显式预加载（Eager Loading）
let blogs = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;

// ✅ 方式二：开启 Lazy Loading 后按需加载
options.use_lazy_loading(true);
let blogs = ctx.set::<Blog>().query().to_list().await?;
for blog in &blogs {
    let posts = blog.posts.load().await?;  // 首次访问时加载
}
```

## 已知限制

| 限制 | 说明 | 替代方案 |
|------|------|----------|
| 多自引用外键 | 同一实体多个自引用 FK 时，`linq!` 的 `include` 无法正确区分导航属性 | 对第二个 FK 使用 `#[foreign_key]`，导航数据手动二次查询 |
| 无 COUNT(DISTINCT) | 框架暂无内置 API | 使用 `group_by` + 内存计数，或通过 provider 执行原始 SQL |
| 无 Form A 的 GROUP BY + 聚合 | 复杂聚合需用 Form B 或内存计数 | 使用 `linq!` Form B 的 `group_by` + `count` 子句 |
| linq! 类型标注 | 闭包参数必须显式标注类型 | `linq!(|b: Blog| ...)` 而非 `linq!(|b| ...)` |