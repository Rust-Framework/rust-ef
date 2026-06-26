# 第一个 CRUD 流程

本节展示完整的增删改查生命周期。

## 创建（Create）

```rust
ctx.set::<Blog>().add(Blog {
    blog_id: 0,          // 自增主键，INSERT 后自动回填
    url: "https://example.com".into(),
    rating: 5,
});

let result = ctx.save_changes().await?;
println!("Added {} blog(s)", result.added);
```

## 查询（Read）

```rust
// 查询全部
let all = ctx.set::<Blog>().query().to_list().await?;

// 推荐写法：拆分为清晰的步骤
let expr = linq!(|b: Blog| b.rating > 3);
let filtered = ctx.set::<Blog>().filter(expr).to_list().await?;

// 按主键查找（使用实体 PK 元数据，不再硬编码 "id"）
let blog = ctx.set::<Blog>().query().find(1).await?;
```

## 更新（Update）

```rust
// 加载 -> 修改 -> 显式 update -> SaveChanges
let mut blog = ctx.set::<Blog>().query().find(1).await?.unwrap();
blog.rating = 10;

ctx.set::<Blog>().update(blog);
ctx.save_changes().await?;
```

## 删除（Delete）

```rust
let mut set = ctx.set::<Blog>();
let blogs = set.query().to_list().await?;

// 按条件标记删除
for (i, blog) in blogs.iter().enumerate() {
    if blog.rating < 2 {
        set.remove_at(i)?;
    }
}

ctx.save_changes().await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| `save_changes()` 是事务边界 | 所有已跟踪变更在一个事务内提交 |
| 自增主键回填 | INSERT 后 `blog_id` 自动更新为数据库生成值 |
| `update()` 显式标记 | 修改实体后必须调用 `update()` 或重新 `attach` |
| `find(id)` 用 PK 元数据 | 不再硬编码 `"id"`，复合主键用 `find_by_key` |

下一章：[实体设计](../03-entity-design/INDEX.md)
