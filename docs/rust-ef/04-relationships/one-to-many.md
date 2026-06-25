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
ctx.set::<Blog>().add(Blog {
    blog_id: 0,
    url: "https://example.com".into(),
    posts: HasMany::new(),
});
ctx.save_changes().await?;

let blog_id = ctx.set::<Blog>().query().to_list().await?[0].blog_id;

ctx.set::<Post>().add(Post {
    post_id: 0,
    title: "Hello".into(),
    blog_id,
    blog: BelongsTo::new(),
});
ctx.save_changes().await?;
```

> 注意：导航属性的物化（填充）需要通过 `include_named` 显式加载，参见 [Eager Loading](eager-loading.md)。

下一节：[多对多与 Join 实体](many-to-many.md)
