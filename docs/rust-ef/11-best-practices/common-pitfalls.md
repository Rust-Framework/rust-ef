# 常见陷阱与排查

## 1. `set::<T>()` 返回 `&mut DbSet<T>`，但 `query()` 只需要 `&self`

```rust
// ✅ 正确：query() 不修改 DbSet 状态
let all = ctx.set::<Blog>().query().to_list().await?;

// ✅ 正确：add() 修改 DbSet 状态
ctx.set::<Blog>().add(blog);
```

## 2. 修改实体后忘记调用 `update()`

```rust
// ❌ 错误：save_changes 不会提交这个修改
let mut blog = ctx.set::<Blog>().query().find_by_id(1).first().await?;
blog.rating = 99;
ctx.save_changes().await?;  // 没有任何 UPDATE！

// ✅ 正确：显式标记修改
ctx.set::<Blog>().update(blog);
ctx.save_changes().await?;
```

## 3. `linq!` 忘记类型标注

```rust
// ❌ 编译错误
let expr = linq!(|b| b.rating > 5);

// ✅ 正确
let expr = linq!(|b: Blog| b.rating > 5);
```

## 4. `ensure_created()` 在 `set::<T>()` 之前调用

```rust
// ❌ 错误：不知道要建哪些表
ctx.ensure_created().await?;
ctx.set::<Blog>();

// ✅ 正确
ctx.set::<Blog>();
ctx.ensure_created().await?;
```

## 5. 在循环里逐条 `save_changes()`

```rust
// ❌ 性能极差，每次循环都开事务
for blog in blogs {
    ctx.set::<Blog>().add(blog);
    ctx.save_changes().await?;
}

// ✅ 正确：一次事务提交全部
for blog in blogs {
    ctx.set::<Blog>().add(blog);
}
ctx.save_changes().await?;
```

## 6. 导航属性为空因为没 `Include`

```rust
// ❌ posts 为空
let blogs = ctx.set::<Blog>().query().to_list().await?;

// ✅ 正确：显式预加载
let blogs = ctx.set::<Blog>().query().include_named("posts").to_list().await?;
```

## 7. `execute_delete()` 误删全表

```rust
// ⚠️ 危险：无过滤条件会删除全表
ctx.set::<Blog>().query().execute_delete().await?;

// ✅ 正确：始终加过滤条件
ctx.set::<Blog>()
    .query()
    .filter(linq!(|b: Blog| b.rating < 1))
    .execute_delete()
    .await?;
```

## 排查流程

```
遇到错误 → 读错误消息 → 查本表 → 查对应章节 → 参考 blog 示例源码
```

## 小结

90% 的问题集中在：忘记 `update()`、`linq!` 类型标注、`set` 与 `ensure_created` 顺序、导航未 `Include`。掌握这 7 条可避免大部分陷阱。

下一节：[性能优化技巧](performance-tips.md)
