# 一对多与 BelongsTo / HasMany

`rust-ef` 提供两种导航类型：`BelongsTo<T>`（多对一）和 `HasMany<T>`（一对多）。

## 定义关系

```rust
#[derive(Debug, Clone, EntityType)]
#[table("blogs")]
pub struct Blog {
    #[primary_key]
    #[auto_increment]
    pub blog_id: i32,
    pub url: String,

    #[navigation]
    pub posts: HasMany<Post>,
}

#[derive(Debug, Clone, EntityType)]
#[table("posts")]
pub struct Post {
    #[primary_key]
    #[auto_increment]
    pub post_id: i32,
    pub title: String,

    #[foreign_key(Blog)]
    pub blog_id: i32,

    #[navigation]
    pub blog: BelongsTo<Blog>,
}
```

## 关键点

| 点 | 说明 |
|---|------|
| `#[foreign_key(Blog)]` | 声明外键列，derive 宏自动提取关联元数据 |
| `#[navigation]` | 标记非持久化导航属性 |
| `HasMany::new()` / `BelongsTo::new()` | 创建空导航容器 |

## 使用导航

```rust
// 创建父实体和子实体
ctx.add::<Blog>(Blog {
    blog_id: 0,
    url: "https://example.com".into(),
    posts: HasMany::new(),
});
ctx.save_changes().await?;

let blog_id = ctx.set::<Blog>().query().to_list().await?[0].blog_id;

ctx.add::<Post>(Post {
    post_id: 0,
    title: "Hello".into(),
    blog_id,
    blog: BelongsTo::new(),
});
ctx.save_changes().await?;
```

> 注意：导航属性的物化（填充）需要通过 `linq!(...; include b.x)` 显式加载，参见 [Eager Loading](eager-loading.md)。

## 单侧声明与同批保存的插入顺序（EF-16，2.4.0）

`save_changes()` 按**类型级依赖图**的拓扑序执行 INSERT，同一个事务里先插 principal（父），再插 dependent（子）。
依赖边来自三类元数据：

| 来源 | 边方向 | 说明 |
|---|---|---|
| `HasMany` / `ManyToMany` 导航 | `related`（子）→ 声明方（父） | 原有行为 |
| `BelongsTo` 导航 | 声明方（子）→ `related`（父） | 2.4.0 新增：当前实体是 dependent，方向与 `HasMany` 相反 |
| 外键元数据（`#[foreign_key(Target)]` / 标量列、`has_foreign_key` 复合键） | 当前实体（子）→ `Target` | 2.4.0 新增：按被引用表名反查 principal 的 `TypeId`（排除 view / keyless；表名不唯一或未注册则跳过） |

因此**只在子侧声明关系**（父侧没有 `HasMany<Child>`）也能在单次 `save_changes()` 中正确排序：

```rust
// Blog 侧没有 HasMany<Post>；Post 侧同时有 #[foreign_key(Blog)] + BelongsTo<Blog>
ctx.add::<Blog>(Blog { blog_id: 0, url: "https://example.com".into() });   // 无 posts 字段
ctx.add::<Post>(Post { post_id: 0, title: "Hello".into(), blog_id: 1, blog: BelongsTo::new() });
ctx.save_changes().await?;   // Blog 先 INSERT，Post 后 INSERT；SQLite/PG/MySQL 均不会 FK 失败
```

实现细节与边界：

- **边去重**：同一对类型即使被 `BelongsTo` 与 `#[foreign_key]` 同时声明也只产生一条边（否则拓扑排序的
  in-degree 与减计数不对称，会静默退化为元数据顺序）。
- **自引用 / 环**：类型级自边不进 in-degree，环沿用既有降级策略（无法定序的节点按元数据顺序追加）；
  同一类型内的实例顺序由 cascade drain + FK fixup 处理。
- **`HasOne` 不产生边**：它的外键在另一侧，单凭该导航不足以推出类型级依赖。
- 外键按**表名**反查 principal；`is_view` / `is_keyless` 的实体不是可插入表，不参与反查。

下一节：[多对多与 Join 实体](many-to-many.md)
