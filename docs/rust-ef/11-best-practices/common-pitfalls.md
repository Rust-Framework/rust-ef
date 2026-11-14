# 常见陷阱与排查

## 1. `set::<T>()` 返回 `&mut DbSet<T>`，但 `query()` 只需要 `&self`

```rust
// ✅ 正确：query() 不修改 DbSet 状态
let all = ctx.set::<Blog>().query().to_list().await?;

// ✅ 正确：add() 修改 DbContext 跟踪状态
ctx.add::<Blog>(blog);
```

## 2. 修改实体后忘记调用 `update()`

```rust
// ❌ 错误：save_changes 不会提交这个修改
let mut blog = ctx.set::<Blog>().query().find(1).await?.unwrap();
blog.rating = 99;
ctx.save_changes().await?;  // 没有任何 UPDATE！

// ✅ 正确：显式标记修改
ctx.update::<Blog>(blog);
ctx.save_changes().await?;
```

## 3. `linq!` 忘记类型标注

```rust
// ❌ 编译错误
let expr = linq!(|b| b.rating > 5);

// ✅ 正确
let expr = linq!(|b: Blog| b.rating > 5);
```

**原因**：`linq!` 是 `proc_macro`（函数式过程宏），在编译的**解析后、类型检查前**阶段执行。宏需要实体类型来将字段引用（如 `b.rating`）编译为列常量（如 `Blog::COLUMN_RATING`），但此阶段类型检查尚未运行，宏无法从上下文推断闭包参数的类型。这是 Rust 过程宏系统的根本限制，1.0 不会改变。Form B 的 source 也必须含 `::<Type>` turbofish，原因相同。

## 4. `ensure_created()` 找不到实体

**原因**：实体没有注册。`DbContext::from_options()` 会自动发现 `#[derive(EntityType)]` 标注且 `context_key` 匹配的实体；未派生、`#[context]` 与当前 key 不一致，或手动构造 `ModelBuilder` 后未 `set::<T>()`，都会得到 `No entity types registered`。

```rust
// from_options 自动发现 + 应用 #[entity(T)] 配置
let mut ctx = DbContext::from_options(&options)?;
ctx.ensure_created().await?;
```

- 手动 `ctx.set::<T>()` 仍可用；`discover_entities()` 仍兼容（幂等空操作），但不是必需步骤。
- `ensure_created()` 通过 `model_builder.build()` 应用所有 Fluent API 与 `#[entity(T)]` 配置覆盖。

**调试技巧**：
- 使用 `ctx.entity_metas_contains::<Blog>()` 检查实体是否已发现
- 使用 `ctx.model_builder().build()` 检查最终的 `EntityTypeMeta` 列表

## 5. 在循环里逐条 `save_changes()`

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

## 6. 导航属性为空因为没 `include`

```rust
// ❌ posts 为空（Lazy Loading 未开启时）
let blogs = ctx.set::<Blog>().query().to_list().await?;

// ✅ 方式一：用 linq! 的 include 子句显式预加载（Eager Loading）
let blogs = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;

// ✅ 方式二：开启 Lazy Loading 后按需加载
let mut options = DbContextOptionsBuilder::new();
options.use_sqlite("app.db").use_lazy_loading(true);
let mut ctx = DbContext::from_options(&options.build())?;

let blogs = ctx.set::<Blog>().query().to_list().await?;
for blog in &blogs {
    let posts = blog.posts.load().await?;  // 首次访问时加载
}
```

详见 [Eager Loading](../04-relationships/eager-loading.md) 与 [Lazy Loading](../04-relationships/lazy-loading.md)。

## 7. `execute_delete()` 误删全表

```rust
// ⚠️ 危险：无过滤条件会删除全表
ctx.set::<Blog>().query().execute_delete().await?;

// ✅ 正确：始终加过滤条件
let affected = linq!(ctx.set::<Blog>(), |b: Blog| b.rating < 1)
    .execute_delete()
    .await?;
```

## 8. 查询形式的 source 用裸变量

```rust
// ❌ 错误：宏无法从变量推断实体类型（需要 turbofish）
let set = ctx.set::<Blog>();
linq!(set; order_by b.rating desc)  // 编译错误

// ✅ 正确：source 必须含 turbofish ::<Type>
linq!(ctx.set::<Blog>(); order_by b.rating desc)
```

## 9. 使用字符串 API

```rust
// ❌ 这些方法已全部移除：include_named / order_by("col") / sum("col") / find_by_id 等
let blogs = ctx.set::<Blog>().query().include_named("posts").to_list().await?;
let blog = ctx.set::<Blog>().query().find_by_id(1).first().await?;
let total = ctx.set::<Blog>().query().sum("views").await?;

// ✅ 正确：统一用 linq! 宏
let blogs = linq!(ctx.set::<Blog>(); include b.posts).to_list().await?;
let blog = ctx.set::<Blog>().query().find(1).await?;
let total: f64 = linq!(ctx.set::<Blog>(); sum b.views).await?;
```

## 10. 只看到 `db error`，定位不到失败语句（2.5.0）

2.5.0 之前驱动错误被压成 `EFError::Query("Execution error: db error")`，丢失 SQLSTATE、表/列/约束和 SQL。
现在 PostgreSQL/MySQL/SQLite 的驱动错误映射为 `EFError::Database(DatabaseErrorInfo)`：

```rust
match err {
    EFError::UniqueViolation { constraint, table } => { /* 唯一冲突：typed，元数据不外泄 */ }
    EFError::Database(info, _) => {
        // 结构化：info.sqlstate / code / message / detail / hint /
        //         table / column / constraint / sql（≤512 字符）
        tracing::error!(
            sqlstate = ?info.sqlstate,
            code = ?info.code,
            table = ?info.table,
            column = ?info.column,
            constraint = ?info.constraint,
            sql = ?info.sql,
            "statement failed"
        );
    }
    _ => {}
}
```

- 客户端仍看不到这些细节：`webx` 映射把 `Database` 归为 500 + `public_detail() = None`，只进日志。
- 唯一/主键冲突仍优先走 `UniqueViolation`，不要改成字符串嗅探。
- 事务/BEGIN/SAVEPOINT 失败也走同一映射，`Display` 与 `source()` 链都保留。

## 排查流程

```
遇到错误 → 读错误消息 → 查本表 → 查对应章节 → 参考 blog 示例源码
```

## 小结

90% 的问题集中在：忘记 `update()`、`linq!` 类型标注、`set` 与 `ensure_created` 顺序、导航未 `include`、使用字符串 API、只读 `db error`。掌握这 10 条可避免大部分陷阱。

下一节：[性能优化技巧](performance-tips.md)
